//! Managed SRT listener. FFmpeg remuxes transport packets into the same
//! timestamped FLV packet path used by an RTMP publisher.
use crate::ingest::{Input, Publisher};
use std::{
    net::SocketAddr,
    path::Path,
    process::Stdio,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, BufReader},
    process::Command,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn start(input: &Arc<Input>, address: SocketAddr, shutdown: &CancellationToken) {
    input.start_managed(shutdown, |cancel| {
        tokio::spawn(run(input.clone(), address, cancel))
    });
}

async fn run(input: Arc<Input>, address: SocketAddr, cancel: CancellationToken) {
    let executable = loop {
        let executable = tokio::select! {
            _ = cancel.cancelled() => return,
            executable = crate::ffmpeg::srt_executable() => executable,
        };
        if let Some(executable) = executable {
            break executable;
        }
        *input.error.lock().unwrap() = Some("No SRT-capable FFmpeg executable is available".into());
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    };
    let url = format!(
        "srt://{address}?mode=listener&transtype=live&enforced_encryption=1&pbkeylen=16&timeout=5000000&passphrase={}",
        input.config.key
    );
    while !cancel.is_cancelled() {
        let result = receive(input.clone(), &executable, &url, &cancel).await;
        if cancel.is_cancelled() {
            break;
        }
        *input.error.lock().unwrap() = result.err();
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

async fn receive(
    input: Arc<Input>,
    executable: &Path,
    url: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut child = Command::new(executable)
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-max_alloc",
            "67108864",
            "-probesize",
            "262144",
            "-analyzeduration",
            "1000000",
            "-copyts",
            "-f",
            "mpegts",
            "-i",
            url,
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c",
            "copy",
            "-flush_packets",
            "1",
            "-flvflags",
            "no_duration_filesize",
            "-f",
            "flv",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Could not start the FFmpeg SRT receiver".to_owned())?;
    let mut stdout = BufReader::new(child.stdout.take().expect("piped SRT receiver output"));
    let result = tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        result = ingest(input, &mut stdout, cancel) => result,
    };
    if child.try_wait().is_ok_and(|status| status.is_none()) {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
    result
}

async fn ingest(
    input: Arc<Input>,
    stream: &mut (impl tokio::io::AsyncRead + Unpin),
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut file_header = [0; 13];
    stream.read_exact(&mut file_header).await.map_err(|_| {
        "SRT receiver stopped before receiving media. Check the UDP port, passphrase and MPEG-TS format."
    })?;
    if &file_header[..3] != b"FLV" || file_header[5..9] != [0, 0, 0, 9] {
        return Err("Unexpected container from the SRT receiver".into());
    }
    let publisher = Publisher::start(input.clone(), cancel)?;
    input.connected.store(true, Ordering::Release);
    let mut tag = [0; 11];
    let mut payload = Vec::new();
    loop {
        if stream.read_exact(&mut tag).await.is_err() {
            return Ok(());
        }
        let size = u32::from_be_bytes([0, tag[1], tag[2], tag[3]]) as usize;
        if size > 2 * 1024 * 1024 {
            return Err("SRT packet exceeds 2 MiB".into());
        }
        payload.resize(size + 4, 0);
        stream
            .read_exact(&mut payload)
            .await
            .map_err(|_| "Incomplete SRT media packet")?;
        if matches!(tag[0], 8 | 9) {
            let timestamp = u32::from_be_bytes([tag[7], tag[4], tag[5], tag[6]]);
            publisher
                .packet(tag[0], timestamp, &payload[..size])
                .await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{self, Runtime};
    use acheflow_media_api::{live::ControlRequest, visual::SourceKind, workspace::Workspace};
    use axum::{Json, extract::State};

    #[tokio::test]
    async fn unsupported_ffmpeg_keeps_the_source_visible_without_starting_a_receiver() {
        let mut workspace = Workspace::default();
        workspace.add_source();
        workspace.sources[0].kind = SourceKind::Srt { port: 9000 };
        let runtime = Runtime::new(Default::default(), "127.0.0.1:1935".into());
        let Json(status) = live::configure(State(runtime.clone()), Json(workspace))
            .await
            .unwrap();
        assert_eq!(
            status.sources[0].error.as_deref(),
            Some("SRT listener is unavailable on this server")
        );
        assert!(
            runtime.inputs.lock().unwrap()[&1]
                .managed
                .lock()
                .unwrap()
                .is_none()
        );
        runtime.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires SRT-enabled FFmpeg and loopback UDP"]
    async fn real_srt_ingest_decodes_video_audio_and_releases_listener_on_deactivate() {
        let caps = crate::ffmpeg::discover().await;
        assert!(
            caps.srt_input,
            "Select an SRT-enabled FFmpeg via ACHEFLOW_FFMPEG"
        );
        let address = {
            let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            socket.local_addr().unwrap()
        };
        let mut workspace = Workspace::default();
        workspace.add_source();
        workspace.sources[0].kind = SourceKind::Srt {
            port: address.port(),
        };
        let key = workspace.sources[0].key.clone();
        let id = workspace.id.clone();
        let runtime = Runtime::new(
            Arc::new(tokio::sync::Mutex::new(caps)),
            "127.0.0.1:1935".into(),
        );
        let _ = live::configure(State(runtime.clone()), Json(workspace))
            .await
            .unwrap();
        let _shutdown = runtime.shutdown.clone().drop_guard();
        let input = runtime.inputs.lock().unwrap()[&1].clone();
        let mut sender = Command::new(crate::ffmpeg::srt_executable().await.unwrap())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-re",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=160x90:r=25",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:sample_rate=48000",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-tune",
                "zerolatency",
                "-g",
                "25",
                "-c:a",
                "aac",
                "-f",
                "mpegts",
            ])
            .arg(format!(
                "srt://{address}?mode=caller&pbkeylen=16&passphrase={key}"
            ))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let frame = input.frame.read().unwrap().clone();
                let has_pts = frame.as_ref().is_some_and(|frame| {
                    frame.pts.is_some() && frame.image.dimensions() == (160, 90)
                });
                if has_pts && input.has_audio.load(Ordering::Acquire) {
                    break;
                }
                assert!(
                    sender.try_wait().unwrap().is_none(),
                    "SRT publisher stopped"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("SRT media missing: {:?}", input.status()));
        let Json(status) = live::deactivate(
            State(runtime.clone()),
            Json(ControlRequest { workspace_id: id }),
        )
        .await
        .unwrap();
        assert!(status.sources.is_empty());
        let _ = sender.kill().await;
        tokio::net::UdpSocket::bind(address)
            .await
            .expect("SRT receiver released its UDP port");
        runtime.close().await;
    }
}
