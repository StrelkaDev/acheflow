use acheflow_media_api::{live::SourceStatus, workspace::Source};
use image::RgbImage;
use std::{
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdout},
    sync::{broadcast, mpsc},
};
use tokio_util::sync::CancellationToken;

pub struct VideoFrame {
    pub received: Instant,
    pub image: Arc<RgbImage>,
}

pub struct Input {
    pub config: Source,
    pub connected: AtomicBool,
    pub frames: AtomicU64,
    pub traffic: Mutex<crate::traffic::Traffic>,
    pub frame: RwLock<Option<VideoFrame>>,
    pub audio: broadcast::Sender<Arc<Vec<i16>>>,
    pub audio_tracks: Mutex<std::collections::BTreeMap<u8, Arc<crate::audio_input::AudioTrack>>>,
    pub has_audio: AtomicBool,
    pub metadata: Mutex<(Option<u32>, Option<u32>, Option<f32>)>,
    pub error: Mutex<Option<String>>,
    pub cancel: Mutex<Option<CancellationToken>>,
}

impl Input {
    pub fn new(config: Source) -> Self {
        let (audio, _) = broadcast::channel(64);
        Self {
            config,
            connected: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            traffic: Mutex::new(Default::default()),
            frame: RwLock::new(None),
            audio,
            audio_tracks: Mutex::new(std::collections::BTreeMap::new()),
            has_audio: AtomicBool::new(false),
            metadata: Mutex::new((None, None, None)),
            error: Mutex::new(None),
            cancel: Mutex::new(None),
        }
    }
    pub fn status(&self) -> SourceStatus {
        let (mut width, mut height, fps) = *self.metadata.lock().unwrap();
        if let Some(frame) = self.frame.read().unwrap().as_ref() {
            width = Some(frame.image.width());
            height = Some(frame.image.height());
        }
        let (received_bytes, bitrate_kbps) = self.traffic.lock().unwrap().snapshot();
        SourceStatus {
            received_bytes,
            bitrate_kbps: if self.connected.load(Ordering::Relaxed) {
                bitrate_kbps
            } else {
                0.
            },
            id: self.config.id,
            connected: self.connected.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            width,
            height,
            fps,
            audio: self.has_audio.load(Ordering::Relaxed),
            audio_tracks: self
                .audio_tracks
                .lock()
                .unwrap()
                .values()
                .map(|t| t.status())
                .collect(),
            error: self.error.lock().unwrap().clone(),
        }
    }
    pub fn disconnect(&self) {
        if let Some(cancel) = self.cancel.lock().unwrap().as_ref() {
            cancel.cancel();
        }
    }
}

pub struct Publisher {
    pub input: Arc<Input>,
    pub cancel: CancellationToken,
    video: mpsc::Sender<Vec<u8>>,
    audio: crate::audio_input::AudioInputs,
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.cancel.cancel();
        *self.input.frame.write().unwrap() = None;
        self.input.has_audio.store(false, Ordering::Release);
        self.input.audio_tracks.lock().unwrap().clear();
        *self.input.cancel.lock().unwrap() = None;
        self.input.connected.store(false, Ordering::Release);
    }
}

impl Publisher {
    pub fn start(input: Arc<Input>, parent: &CancellationToken) -> Result<Self, String> {
        let cancel = parent.child_token();
        let video = decoder(input.clone(), cancel.clone())?;
        let audio = crate::audio_input::AudioInputs::new(input.clone(), cancel.clone());
        *input.error.lock().unwrap() = None;
        *input.metadata.lock().unwrap() = (None, None, None);
        input.frames.store(0, Ordering::Relaxed);
        *input.cancel.lock().unwrap() = Some(cancel.clone());
        Ok(Self {
            input,
            cancel,
            video,
            audio,
        })
    }
    pub async fn packet(&self, kind: u8, timestamp: u32, data: &[u8]) -> Result<(), String> {
        if data.len() > 2 * 1024 * 1024 {
            return Err("RTMP packet exceeds 2 MiB".into());
        }
        self.input.traffic.lock().unwrap().record(data.len() as u64);
        if kind == 8 {
            return self
                .audio
                .packet(timestamp, data)
                .await
                .inspect_err(|e| *self.input.error.lock().unwrap() = Some(e.clone()));
        }
        let tag = flv_tag(kind, timestamp, data);
        let sender = &self.video;
        tokio::time::timeout(std::time::Duration::from_secs(2), sender.send(tag))
            .await
            .map_err(|_| "Input decoder is overloaded".to_owned())?
            .map_err(|_| "Input decoder stopped".to_owned())
    }
}

pub(crate) fn flv_tag(kind: u8, timestamp: u32, data: &[u8]) -> Vec<u8> {
    let mut tag = Vec::with_capacity(data.len() + 15);
    tag.push(kind);
    tag.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
    tag.extend_from_slice(&timestamp.to_be_bytes()[1..]);
    tag.push((timestamp >> 24) as u8);
    tag.extend_from_slice(&[0, 0, 0]);
    tag.extend_from_slice(data);
    tag.extend_from_slice(&(data.len() as u32 + 11).to_be_bytes());
    tag
}

fn decoder(input: Arc<Input>, cancel: CancellationToken) -> Result<mpsc::Sender<Vec<u8>>, String> {
    use std::process::Stdio;
    let mut command = tokio::process::Command::new(crate::ffmpeg::executable());
    command.env("AV_LOG_FORCE_NOCOLOR", "1");
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-nostats",
        "-threads",
        "2",
        "-max_alloc",
        "67108864",
        "-max_pixels",
        "8294400",
        "-probesize",
        "32768",
        "-analyzeduration",
        "100000",
        "-fpsprobesize",
        "0",
        "-f",
        "flv",
        "-i",
        "pipe:0",
    ]);
    // Raw RGB supports changing dimensions. showinfo supplies one size per
    // frame after pixel conversion; a bounded queue pairs it with stdout.
    command.args([
        "-loglevel",
        "level+info",
        "-map",
        "0:v:0",
        "-an",
        "-fps_mode",
        "passthrough",
        "-threads:v",
        "1",
        "-noautoscale",
        "-vf",
        "format=pix_fmts=rgb24,showinfo@acheflow=checksum=0",
        "-pix_fmt",
        "rgb24",
        "-c:v",
        "rawvideo",
        "-f",
        "rawvideo",
        "pipe:1",
    ]);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Cannot start input decoder: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("Missing decoder stdin")?;
    let stdout = child.stdout.take().ok_or("Missing decoder stdout")?;
    let stderr = child.stderr.take().ok_or("Missing decoder stderr")?;
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(8);
    let (sizes_tx, sizes_rx) = mpsc::channel(8);
    tokio::spawn(async move {
        let writer = tokio::spawn(async move {
            stdin
                .write_all(&[b'F', b'L', b'V', 1, 1, 0, 0, 0, 9, 0, 0, 0, 0])
                .await?;
            while let Some(tag) = rx.recv().await {
                stdin.write_all(&tag).await?;
            }
            Ok::<_, std::io::Error>(())
        });
        let read_input = input.clone();
        let read_cancel = cancel.clone();
        let reader = tokio::spawn(async move {
            let result = read_video(read_input.clone(), stdout, sizes_rx).await;
            if let Err(error) = &result
                && error.kind() != std::io::ErrorKind::UnexpectedEof
            {
                *read_input.error.lock().unwrap() = Some(format!("Input decoder: {error}"));
            }
            read_cancel.cancel();
            result
        });
        let errors = tokio::spawn(read_decoder_events(input.clone(), stderr, sizes_tx));
        reap(&mut child, &cancel).await;
        writer.abort();
        reader.abort();
        errors.abort();
        cancel.cancel();
    });
    Ok(tx)
}

pub async fn reap(child: &mut Child, cancel: &CancellationToken) {
    tokio::select! {
        _=cancel.cancelled()=> {let _=child.kill().await;let _=child.wait().await;}
        _=child.wait()=> {}
    }
}

const MAX_INPUT_PIXELS: u64 = 8_294_400;

async fn read_decoder_events(
    input: Arc<Input>,
    stderr: ChildStderr,
    sizes: mpsc::Sender<Result<(u32, u32), String>>,
) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(size) = frame_size(&line) {
            if sizes.send(size).await.is_err() {
                break;
            }
        } else if !line.trim().is_empty() && (line.contains("[error]") || line.contains("[fatal]"))
        {
            *input.error.lock().unwrap() = Some(line.chars().take(400).collect());
        }
    }
}

fn frame_size(line: &str) -> Option<Result<(u32, u32), String>> {
    if !line.starts_with("[showinfo@acheflow @ ") || !line.contains(" n:") {
        return None;
    }
    Some(
        (|| {
            if !line.contains("fmt:rgb24 ") {
                return None;
            }
            let size = line.split_once(" s:")?.1.split_whitespace().next()?;
            let (width, height) = size.split_once('x')?;
            Some((width.parse().ok()?, height.parse().ok()?))
        })()
        .ok_or_else(|| "Invalid FFmpeg frame dimensions".into()),
    )
}

fn invalid_frame(message: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.to_string())
}

async fn read_rgb_frame(
    reader: &mut (impl AsyncRead + Unpin),
    (width, height): (u32, u32),
) -> std::io::Result<RgbImage> {
    if width == 0
        || height == 0
        || width > 7680
        || height > 7680
        || u64::from(width) * u64::from(height) > MAX_INPUT_PIXELS
    {
        return Err(invalid_frame("Unsupported input frame size or pixel depth"));
    }
    let mut bytes = vec![0; width as usize * height as usize * 3];
    reader.read_exact(&mut bytes).await?;
    RgbImage::from_raw(width, height, bytes)
        .ok_or_else(|| invalid_frame("Invalid RGB frame length"))
}

async fn read_video(
    input: Arc<Input>,
    stdout: ChildStdout,
    mut sizes: mpsc::Receiver<Result<(u32, u32), String>>,
) -> std::io::Result<()> {
    let mut stdout = BufReader::new(stdout);
    while let Some(size) = sizes.recv().await {
        let image = read_rgb_frame(&mut stdout, size.map_err(invalid_frame)?).await?;
        input.frames.fetch_add(1, Ordering::Relaxed);
        *input.frame.write().unwrap() = Some(VideoFrame {
            received: Instant::now(),
            image: Arc::new(image),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rgb_frame_boundaries_follow_each_frames_dimensions() {
        let mut stream = Vec::new();
        for (width, height, value) in [(2, 1, b'\n'), (1, 3, b'P')] {
            stream.extend(vec![value; width * height * 3]);
        }
        let mut reader = stream.as_slice();
        let first = read_rgb_frame(&mut reader, (2, 1)).await.unwrap();
        let second = read_rgb_frame(&mut reader, (1, 3)).await.unwrap();
        assert_eq!(first.dimensions(), (2, 1));
        assert!(first.as_raw().iter().all(|b| *b == b'\n'));
        assert_eq!(second.dimensions(), (1, 3));
        assert!(second.as_raw().iter().all(|b| *b == b'P'));
        assert!(reader.is_empty());
    }

    #[tokio::test]
    async fn invalid_frames_are_rejected_before_pixel_allocation() {
        for size in [
            (0, 2),
            (2, 0),
            (7681, 1),
            (3840, 2162),
            (u32::MAX, u32::MAX),
        ] {
            assert_eq!(
                read_rgb_frame(&mut &b""[..], size)
                    .await
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidData,
                "{size:?}"
            );
        }
        assert_eq!(
            read_rgb_frame(&mut &b"123"[..], (2, 2))
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn only_video_frame_descriptors_supply_rgb_dimensions() {
        assert_eq!(
            frame_size(
                "[showinfo@acheflow @ 0x123] [info] n: 0 pts:0 fmt:rgb24 sar:1/1 s:160x90 i:P iskey:1"
            ),
            Some(Ok((160, 90)))
        );
        assert_eq!(
            frame_size("[showinfo@acheflow @ 0x123] [info] config in time_base: 1/1000"),
            None
        );
        assert_eq!(frame_size("publisher: n:0 fmt:rgb24 s:1x1"), None);
        assert!(
            frame_size("[showinfo@acheflow @ 0x123] [info] n: 0 fmt:rgb24 s:broken")
                .unwrap()
                .is_err()
        );
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264; run explicitly"]
    async fn real_decoder_handles_resolution_changes_without_restarting() {
        let mut workspace = acheflow_media_api::workspace::Workspace::default();
        workspace.add_source();
        let input = Arc::new(Input::new(workspace.sources.remove(0)));
        let cancel = CancellationToken::new();
        let tx = decoder(input.clone(), cancel.clone()).unwrap();
        for (index, (width, height)) in [(160, 90), (90, 160), (320, 180)].into_iter().enumerate() {
            let fixture = tokio::process::Command::new(crate::ffmpeg::executable())
                .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
                .arg(format!("testsrc2=s={width}x{height}:r=10"))
                .args([
                    "-frames:v",
                    "12",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-tune",
                    "zerolatency",
                ])
                .arg("-output_ts_offset")
                .arg(format!("{}", index as f64 * 1.2))
                .args(["-f", "flv", "pipe:1"])
                .kill_on_drop(true)
                .output()
                .await
                .unwrap();
            assert!(
                fixture.status.success(),
                "{}",
                String::from_utf8_lossy(&fixture.stderr)
            );
            // The decoder supplies its own FLV header; subsequent AVC sequence
            // headers describe each new size within the same decoder process.
            tx.send(fixture.stdout[13..].to_vec()).await.unwrap();
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if input
                        .frame
                        .read()
                        .unwrap()
                        .as_ref()
                        .is_some_and(|f| f.image.dimensions() == (width, height))
                    {
                        break;
                    }
                    assert!(
                        !cancel.is_cancelled(),
                        "decoder stopped: {:?}",
                        input.error.lock().unwrap()
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await;
            assert!(
                result.is_ok(),
                "expected {width}x{height}, frames={}, current={:?}, error={:?}",
                input.frames.load(Ordering::Relaxed),
                input
                    .frame
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|f| f.image.dimensions()),
                input.error.lock().unwrap()
            );
        }
        cancel.cancel();
    }

    #[test]
    fn flv_tag_preserves_extended_timestamp_and_packet_size() {
        let tag = flv_tag(9, 0x12345678, &[1, 2, 3]);
        assert_eq!(&tag[..11], &[9, 0, 0, 3, 0x34, 0x56, 0x78, 0x12, 0, 0, 0]);
        assert_eq!(&tag[14..], &14u32.to_be_bytes());
    }
}
