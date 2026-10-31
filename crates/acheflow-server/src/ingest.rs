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
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdout},
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
    pub frame: RwLock<Option<VideoFrame>>,
    pub audio: broadcast::Sender<Arc<Vec<i16>>>,
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
            frame: RwLock::new(None),
            audio,
            has_audio: AtomicBool::new(false),
            metadata: Mutex::new((None, None, None)),
            error: Mutex::new(None),
            cancel: Mutex::new(None),
        }
    }
    pub fn status(&self) -> SourceStatus {
        let (width, height, fps) = *self.metadata.lock().unwrap();
        SourceStatus {
            id: self.config.id,
            connected: self.connected.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            width,
            height,
            fps,
            audio: self.has_audio.load(Ordering::Relaxed),
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
    audio: mpsc::Sender<Vec<u8>>,
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.cancel.cancel();
        *self.input.frame.write().unwrap() = None;
        self.input.has_audio.store(false, Ordering::Release);
        *self.input.cancel.lock().unwrap() = None;
        self.input.connected.store(false, Ordering::Release);
    }
}

impl Publisher {
    pub fn start(input: Arc<Input>, parent: &CancellationToken) -> Result<Self, String> {
        let cancel = parent.child_token();
        let video = decoder(input.clone(), true, cancel.clone())?;
        let audio = match decoder(input.clone(), false, cancel.clone()) {
            Ok(audio) => audio,
            Err(error) => {
                cancel.cancel();
                return Err(error);
            }
        };
        *input.error.lock().unwrap() = None;
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
        let tag = flv_tag(kind, timestamp, data);
        let sender = if kind == 9 { &self.video } else { &self.audio };
        tokio::time::timeout(std::time::Duration::from_secs(2), sender.send(tag))
            .await
            .map_err(|_| "Input decoder is overloaded".to_owned())?
            .map_err(|_| "Input decoder stopped".to_owned())
    }
}

fn flv_tag(kind: u8, timestamp: u32, data: &[u8]) -> Vec<u8> {
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

fn decoder(
    input: Arc<Input>,
    video: bool,
    cancel: CancellationToken,
) -> Result<mpsc::Sender<Vec<u8>>, String> {
    use std::process::Stdio;
    let mut command = tokio::process::Command::new(crate::ffmpeg::executable());
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
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
    if video {
        let size = &input.config.resolution;
        command.args(["-map","0:v:0","-an","-vf"])
            .arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1",size.width,size.height,size.width,size.height))
            .args(["-fps_mode","passthrough","-threads:v","1","-pix_fmt","rgb24","-f","rawvideo","pipe:1"]);
    } else {
        command.args([
            "-map",
            "0:a:0",
            "-vn",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
            "-f",
            "s16le",
            "pipe:1",
        ]);
    }
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
    tokio::spawn(async move {
        let writer = tokio::spawn(async move {
            let flags = if video { 1 } else { 4 };
            stdin
                .write_all(&[b'F', b'L', b'V', 1, flags, 0, 0, 0, 9, 0, 0, 0, 0])
                .await?;
            while let Some(tag) = rx.recv().await {
                stdin.write_all(&tag).await?;
            }
            Ok::<_, std::io::Error>(())
        });
        let read_input = input.clone();
        let reader = tokio::spawn(async move {
            if video {
                read_video(read_input, stdout).await
            } else {
                read_audio(read_input, stdout).await
            }
        });
        let error_input = input.clone();
        let errors = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    *error_input.error.lock().unwrap() = Some(line.chars().take(400).collect());
                }
            }
        });
        reap(&mut child, &cancel).await;
        writer.abort();
        reader.abort();
        errors.abort();
        if video {
            cancel.cancel();
        }
    });
    Ok(tx)
}

pub async fn reap(child: &mut Child, cancel: &CancellationToken) {
    tokio::select! {
        _=cancel.cancelled()=> {let _=child.kill().await;let _=child.wait().await;}
        _=child.wait()=> {}
    }
}

async fn read_video(input: Arc<Input>, mut stdout: ChildStdout) -> std::io::Result<()> {
    let size = &input.config.resolution;
    let length = size.width as usize * size.height as usize * 3;
    loop {
        let mut bytes = vec![0; length];
        stdout.read_exact(&mut bytes).await?;
        input.frames.fetch_add(1, Ordering::Relaxed);
        let image =
            RgbImage::from_raw(size.width, size.height, bytes).expect("validated raw frame length");
        *input.frame.write().unwrap() = Some(VideoFrame {
            received: Instant::now(),
            image: Arc::new(image),
        });
    }
}
async fn read_audio(input: Arc<Input>, mut stdout: ChildStdout) -> std::io::Result<()> {
    loop {
        let mut bytes = vec![0u8; 3840];
        stdout.read_exact(&mut bytes).await?;
        let samples = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| i16::from_le_bytes([p[0], p[1]]))
            .collect();
        input.has_audio.store(true, Ordering::Relaxed);
        let _ = input.audio.send(Arc::new(samples));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flv_tag_preserves_extended_timestamp_and_packet_size() {
        let tag = flv_tag(9, 0x12345678, &[1, 2, 3]);
        assert_eq!(&tag[..11], &[9, 0, 0, 3, 0x34, 0x56, 0x78, 0x12, 0, 0, 0]);
        assert_eq!(&tag[14..], &14u32.to_be_bytes());
    }
}
