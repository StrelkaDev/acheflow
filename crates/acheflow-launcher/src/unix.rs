use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const VERSION: u32 = 1;
const FRAME_LIMIT: usize = MAX_FORWARD_URL_BYTES * 6 + 512;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    version: u32,
    recipient: String,
    request: Request,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    version: u32,
    result: Result<(), ActivationError>,
}
fn name(recipient: ForwardRecipient) -> &'static str {
    match recipient {
        ForwardRecipient::Client => "client",
        ForwardRecipient::Server => "server",
    }
}
pub enum Startup {
    Primary(Primary),
    HandedOff,
}
pub struct Primary {
    _lock: File,
    socket: PathBuf,
    inbox: Arc<Inbox>,
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Primary {
    pub fn inbox(&self) -> Arc<Inbox> {
        self.inbox.clone()
    }
}
impl Drop for Primary {
    fn drop(&mut self) {
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // The lock is still held. Never remove a replacement regular file/symlink.
        if private_socket(&self.socket).is_ok() {
            let _ = fs::remove_file(&self.socket);
        }
    }
}
pub fn default_directory() -> Result<PathBuf, ActivationError> {
    Ok(std::env::var_os("ACHEFLOW_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("acheflow-{}", rustix::process::geteuid().as_raw()))
        }))
}
fn private_directory(path: &Path) -> Result<PathBuf, ActivationError> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(ActivationError::UnsafeRuntime),
    }
    let m = fs::symlink_metadata(path).map_err(|_| ActivationError::UnsafeRuntime)?;
    if !m.is_dir()
        || m.uid() != rustix::process::geteuid().as_raw()
        || m.permissions().mode() & 0o077 != 0
    {
        return Err(ActivationError::UnsafeRuntime);
    }
    path.canonicalize()
        .map_err(|_| ActivationError::UnsafeRuntime)
}
fn private_socket(path: &Path) -> Result<(), ActivationError> {
    let m = fs::symlink_metadata(path).map_err(|_| ActivationError::UnsafeRuntime)?;
    if !m.file_type().is_socket()
        || m.uid() != rustix::process::geteuid().as_raw()
        || m.permissions().mode() & 0o077 != 0
    {
        return Err(ActivationError::UnsafeRuntime);
    }
    Ok(())
}
pub fn start(
    recipient: ForwardRecipient,
    request: Request,
    directory: &Path,
) -> Result<Startup, ActivationError> {
    request.validate(recipient)?;
    let directory = private_directory(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(directory.join(format!("{}.lock", name(recipient))))
        .map_err(|_| ActivationError::UnsafeRuntime)?;
    let metadata = lock
        .metadata()
        .map_err(|_| ActivationError::UnsafeRuntime)?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(ActivationError::UnsafeRuntime);
    }
    let socket = directory.join(format!("{}.sock", name(recipient)));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match lock.try_lock() {
            Ok(()) => return primary(lock, socket, recipient, request),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(_) => return Err(ActivationError::Unavailable),
        }
        if socket.exists() {
            private_socket(&socket)?;
            match UnixStream::connect(&socket) {
                Ok(stream) => {
                    send(stream, recipient, request)?;
                    return Ok(Startup::HandedOff);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) => {}
                Err(_) => return Err(ActivationError::Unavailable),
            }
        }
        if Instant::now() >= deadline {
            return Err(ActivationError::Unavailable);
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn primary(
    lock: File,
    socket: PathBuf,
    recipient: ForwardRecipient,
    request: Request,
) -> Result<Startup, ActivationError> {
    if fs::symlink_metadata(&socket).is_ok() {
        private_socket(&socket)?;
        fs::remove_file(&socket).map_err(|_| ActivationError::UnsafeRuntime)?;
    }
    // Publish only the fully private endpoint. Followers must not observe the
    // intermediate bind permissions while the primary is still initializing.
    let staging = socket.with_file_name(match recipient {
        ForwardRecipient::Client => ".c.new",
        ForwardRecipient::Server => ".s.new",
    });
    if let Ok(metadata) = fs::symlink_metadata(&staging) {
        if !metadata.file_type().is_socket()
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(ActivationError::UnsafeRuntime);
        }
        fs::remove_file(&staging).map_err(|_| ActivationError::UnsafeRuntime)?;
    }
    let listener = UnixListener::bind(&staging).map_err(|_| ActivationError::Unavailable)?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o600))
        .map_err(|_| ActivationError::UnsafeRuntime)?;
    listener
        .set_nonblocking(true)
        .map_err(|_| ActivationError::Unavailable)?;
    fs::rename(&staging, &socket).map_err(|_| ActivationError::UnsafeRuntime)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| ActivationError::Unavailable)?;
    let (sender, receiver) = mpsc::channel(INBOX_CAPACITY);
    sender
        .try_send(Activation::initial(request))
        .map_err(|_| ActivationError::Busy)?;
    let inbox = Arc::new(Inbox(Mutex::new(Some(receiver))));
    let (shutdown, stop) = oneshot::channel();
    let worker = thread::Builder::new()
        .name("acheflow-activation".into())
        .spawn(move || runtime.block_on(serve(listener, recipient, sender, stop)))
        .map_err(|_| ActivationError::Unavailable)?;
    Ok(Startup::Primary(Primary {
        _lock: lock,
        socket,
        inbox,
        shutdown: Some(shutdown),
        worker: Some(worker),
    }))
}
fn send(
    mut stream: UnixStream,
    recipient: ForwardRecipient,
    request: Request,
) -> Result<(), ActivationError> {
    use std::io::Read;
    stream
        .set_read_timeout(Some(HANDOFF_TIMEOUT + Duration::from_secs(1)))
        .map_err(|_| ActivationError::Unavailable)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| ActivationError::Unavailable)?;
    let bytes = serde_json::to_vec(&Wire {
        version: VERSION,
        recipient: name(recipient).into(),
        request,
    })
    .map_err(|_| ActivationError::InvalidLink)?;
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(&bytes))
        .map_err(|_| ActivationError::OutcomeUnknown)?;
    let mut length = [0; 4];
    stream
        .read_exact(&mut length)
        .map_err(|_| ActivationError::OutcomeUnknown)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > 512 {
        return Err(ActivationError::OutcomeUnknown);
    }
    let mut bytes = vec![0; length];
    stream
        .read_exact(&mut bytes)
        .map_err(|_| ActivationError::OutcomeUnknown)?;
    let reply: Reply =
        serde_json::from_slice(&bytes).map_err(|_| ActivationError::OutcomeUnknown)?;
    if reply.version != VERSION {
        return Err(ActivationError::OutcomeUnknown);
    }
    reply.result
}
async fn serve(
    listener: UnixListener,
    recipient: ForwardRecipient,
    sender: mpsc::Sender<Activation>,
    mut stop: oneshot::Receiver<()>,
) {
    let Ok(listener) = tokio::net::UnixListener::from_std(listener) else {
        return;
    };
    let mut workers = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _=&mut stop=>break,
            Some(_)=workers.join_next(),if !workers.is_empty()=>{},
            accepted=listener.accept()=>{
                let Ok((mut stream,_))=accepted else{break};
                if workers.len()>=16{let _=write_reply(&mut stream,Err(ActivationError::Busy)).await;continue;}
                let sender=sender.clone();
                workers.spawn(async move {
                    let result=receive(&mut stream,recipient,sender).await;
                    let _=tokio::time::timeout(Duration::from_secs(1),write_reply(&mut stream,result)).await;
                });
            }
        }
    }
}
async fn receive(
    stream: &mut tokio::net::UnixStream,
    recipient: ForwardRecipient,
    sender: mpsc::Sender<Activation>,
) -> Result<(), ActivationError> {
    let wire = tokio::time::timeout(Duration::from_secs(1), async {
        let length = stream
            .read_u32()
            .await
            .map_err(|_| ActivationError::Rejected)? as usize;
        if length > FRAME_LIMIT {
            return Err(ActivationError::Rejected);
        }
        let mut bytes = vec![0; length];
        stream
            .read_exact(&mut bytes)
            .await
            .map_err(|_| ActivationError::Rejected)?;
        serde_json::from_slice::<Wire>(&bytes).map_err(|_| ActivationError::Rejected)
    })
    .await
    .map_err(|_| ActivationError::Expired)??;
    if wire.version != VERSION || wire.recipient != name(recipient) {
        return Err(ActivationError::Rejected);
    }
    wire.request.validate(recipient)?;
    let (reply, result) = oneshot::channel();
    sender
        .try_send(Activation {
            request: wire.request,
            reply: Some(reply),
            deadline: Instant::now() + HANDOFF_TIMEOUT,
        })
        .map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => ActivationError::Busy,
            mpsc::error::TrySendError::Closed(_) => ActivationError::NotReady,
        })?;
    tokio::time::timeout(HANDOFF_TIMEOUT, result)
        .await
        .map_err(|_| ActivationError::OutcomeUnknown)?
        .map_err(|_| ActivationError::OutcomeUnknown)?
}
async fn write_reply(
    stream: &mut tokio::net::UnixStream,
    result: Result<(), ActivationError>,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(&Reply {
        version: VERSION,
        result,
    })
    .map_err(io::Error::other)?;
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await
}
