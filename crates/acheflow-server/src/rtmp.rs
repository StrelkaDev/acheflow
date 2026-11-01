use crate::{ingest::Publisher, live::RuntimeState};
use rml_rtmp::{
    handshake::{Handshake, HandshakeProcessResult, PeerType},
    sessions::{ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult},
};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
};
type Error = Box<dyn std::error::Error + Send + Sync>;

pub async fn serve(
    listener: TcpListener,
    runtime: RuntimeState,
    tls: Option<axum_server::tls_openssl::OpenSSLConfig>,
) -> Result<(), Error> {
    let capacity = Arc::new(Semaphore::new(16));
    loop {
        let (socket, _) = tokio::select! {_=runtime.shutdown.cancelled()=>return Ok(()),result=listener.accept()=>result?};
        let Ok(permit) = capacity.clone().try_acquire_owned() else {
            drop(socket);
            continue;
        };
        let runtime = runtime.clone();
        let tls = tls.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = socket.set_nodelay(true);
            if let Some(tls) = tls {
                let Ok(ssl) = openssl::ssl::Ssl::new(tls.get_inner().context()) else {
                    return;
                };
                let Ok(mut stream) = tokio_openssl::SslStream::new(ssl, socket) else {
                    return;
                };
                let accepted = tokio::select! {
                    _ = runtime.shutdown.cancelled() => return,
                    result = tokio::time::timeout(Duration::from_secs(5), std::pin::Pin::new(&mut stream).accept()) => result,
                };
                if let Ok(Ok(())) = accepted {
                    let _ = connection(stream, runtime).await;
                }
            } else {
                let _ = connection(socket, runtime).await;
            }
        });
    }
}

async fn connection(
    mut socket: impl AsyncRead + AsyncWrite + Unpin,
    runtime: RuntimeState,
) -> Result<(), Error> {
    let cancel = runtime.shutdown.child_token();
    let mut handshake = Handshake::new(PeerType::Server);
    let mut bytes = vec![0; 65536];
    let remaining = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count = socket.read(&mut bytes).await?;
            if count == 0 {
                return Err::<Vec<u8>, Error>("RTMP handshake disconnected".into());
            }
            match handshake.process_bytes(&bytes[..count])? {
                HandshakeProcessResult::InProgress { response_bytes } => {
                    socket.write_all(&response_bytes).await?
                }
                HandshakeProcessResult::Completed {
                    response_bytes,
                    remaining_bytes,
                } => {
                    socket.write_all(&response_bytes).await?;
                    return Ok(remaining_bytes);
                }
            }
        }
    })
    .await??;
    let (mut session, initial) = ServerSession::new(ServerSessionConfig::new())?;
    let mut pending: VecDeque<_> = initial.into();
    pending.extend(session.handle_input(&remaining)?);
    let mut publisher: Option<Publisher> = None;
    loop {
        while let Some(result) = pending.pop_front() {
            match result {
                ServerSessionResult::OutboundResponse(packet) => {
                    socket.write_all(&packet.bytes).await?
                }
                ServerSessionResult::RaisedEvent(event) => match event {
                    ServerSessionEvent::ConnectionRequested {
                        request_id,
                        app_name,
                    } => {
                        if app_name.is_empty() || app_name.len() > 256 {
                            return Err("Invalid RTMP application".into());
                        }
                        pending.extend(session.accept_request(request_id)?);
                    }
                    ServerSessionEvent::PublishStreamRequested {
                        request_id,
                        app_name,
                        stream_key,
                        ..
                    } => {
                        if publisher.is_some() {
                            return Err("One publisher per RTMP connection".into());
                        }
                        match runtime.authorize(&app_name, &stream_key, &cancel) {
                            Ok(input) => {
                                publisher = Some(input);
                                pending.extend(session.accept_request(request_id)?);
                            }
                            Err(_) => {
                                let rejected = session.reject_request(
                                    request_id,
                                    "NetStream.Publish.BadName",
                                    "Invalid token or source already connected",
                                )?;
                                for result in rejected {
                                    if let ServerSessionResult::OutboundResponse(packet) = result {
                                        socket.write_all(&packet.bytes).await?;
                                    }
                                }
                                return Ok(());
                            }
                        }
                    }
                    ServerSessionEvent::PlayStreamRequested { request_id, .. } => {
                        pending.extend(session.reject_request(
                            request_id,
                            "NetStream.Play.StreamNotFound",
                            "This endpoint accepts publishers only",
                        )?)
                    }
                    ServerSessionEvent::PublishStreamFinished { .. } => {
                        publisher.take();
                    }
                    ServerSessionEvent::VideoDataReceived {
                        data, timestamp, ..
                    } => {
                        if let Some(publisher) = &publisher {
                            publisher.packet(9, timestamp.value, &data).await?;
                        }
                    }
                    ServerSessionEvent::AudioDataReceived {
                        data, timestamp, ..
                    } => {
                        if let Some(publisher) = &publisher {
                            publisher.packet(8, timestamp.value, &data).await?;
                        }
                    }
                    ServerSessionEvent::StreamMetadataChanged { metadata, .. } => {
                        if let Some(publisher) = &publisher {
                            *publisher.input.metadata.lock().unwrap() = (
                                metadata.video_width.filter(|w| *w > 0 && *w <= 32768),
                                metadata.video_height.filter(|h| *h > 0 && *h <= 32768),
                                metadata
                                    .video_frame_rate
                                    .filter(|fps| fps.is_finite() && (1.0..=240.0).contains(fps)),
                            );
                        }
                    }
                    ServerSessionEvent::ClientChunkSizeChanged { new_chunk_size }
                        if new_chunk_size > 1024 * 1024 =>
                    {
                        return Err("RTMP chunk too large".into());
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        let published_cancel = publisher
            .as_ref()
            .map(|p| p.cancel.clone())
            .unwrap_or_default();
        let read = tokio::select! {
            _=cancel.cancelled()=>break,
            _=published_cancel.cancelled()=>break,
            result=tokio::time::timeout(Duration::from_secs(if publisher.is_some(){30}else{10}),socket.read(&mut bytes))=>result??,
        };
        if read == 0 {
            break;
        }
        pending.extend(session.handle_input(&bytes[..read])?);
    }
    Ok(())
}
