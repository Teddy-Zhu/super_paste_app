//! A secondary launch forwards only a typed activation, before opening SQLite
//! or the credential store. The advisory lock outlives the IPC listener, even
//! if the listener fails, and is released explicitly before restart handoff.
use std::{fs::{File, OpenOptions}, io, path::Path, sync::Arc, time::Duration};
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Name, traits::StreamCommon as _, tokio::{prelude::*, Listener, Stream}};
use sha2::{Digest, Sha256};
use super_paste_core::{Error, Result};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, runtime::Handle, sync::mpsc, task::{JoinHandle, JoinSet}, time::Instant};
use tokio_util::sync::CancellationToken;
use crate::paths::AppPaths;

const MAGIC: &[u8; 5] = b"SPRS\x01";
const TOKEN_LIMIT: usize = 2048;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const ACTIVATION_LIMIT: usize = 32;
pub(crate) const BACKGROUND_ARGUMENT: &str = "--background";

#[derive(Clone)]
pub enum LaunchRequest { Default, Background, ToggleOverlay { activation_token: Option<String> } }
#[derive(Clone)]
pub enum Activation { ShowSettings, ToggleOverlay { activation_token: Option<String> }, Tray(crate::tray::TrayAction) }
impl LaunchRequest {
    pub fn from_environment() -> Result<Self> {
        let arguments: Vec<_> = std::env::args_os().skip(1).collect();
        if arguments.iter().any(|argument| argument == "--toggle-overlay") {
            let activation_token = std::env::var("XDG_ACTIVATION_TOKEN").ok().filter(|token| !token.is_empty());
            if activation_token.as_ref().is_some_and(|token| !valid_token(token)) { return Err(Error::Invalid("窗口激活令牌无效".into())); }
            Ok(Self::ToggleOverlay { activation_token })
        } else if arguments.iter().any(|argument| argument == BACKGROUND_ARGUMENT) { Ok(Self::Background) }
        else { Ok(Self::Default) }
    }
}
pub enum InstanceRole {
    Primary { instance: SingleInstance, activations: mpsc::Receiver<Activation>, sender: mpsc::Sender<Activation> },
    Forwarded,
}
pub struct SingleInstance {
    lock: Option<Arc<File>>, stop: CancellationToken, server: Option<JoinHandle<Result<()>>>,
}
impl SingleInstance {
    pub async fn acquire(paths: &AppPaths, launch: LaunchRequest, runtime: &Handle) -> Result<InstanceRole> {
        tokio::fs::create_dir_all(&paths.data).await?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&paths.data, std::fs::Permissions::from_mode(0o700)).await?;
        }
        let directory = tokio::fs::canonicalize(&paths.data).await?;
        let name = socket_name(&directory)?;
        let file = tokio::task::spawn_blocking(move || {
            let mut options = OpenOptions::new(); options.create(true).read(true).write(true).truncate(false);
            #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
            let file = options.open(directory.join("instance.lock"))?;
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => Ok(Some(file)),
                Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Ok(None),
                Err(error) => Err(error),
            }
        }).await.map_err(Error::from)??;
        let Some(file) = file else { forward(name, launch).await?; return Ok(InstanceRole::Forwarded); };
        let mut options = ListenerOptions::new().name(name).try_overwrite(true);
        #[cfg(unix)] { use interprocess::os::unix::local_socket::ListenerOptionsExt as _; options = options.mode(0o600).max_spin_time(IO_TIMEOUT); }
        let listener = options.create_tokio().map_err(|error| Error::Platform(format!("建立应用激活入口失败：{error}").into()))?;
        let lock = Arc::new(file); let lock_for_server = lock.clone(); let stop = CancellationToken::new(); let cancelled = stop.clone();
        let (sender, activations) = mpsc::channel(ACTIVATION_LIMIT);
        let server_sender = sender.clone();
        let server = runtime.spawn(async move {
            // The server's reference also keeps the lock alive if its owner is
            // dropped without awaiting shutdown; listener cleanup always wins.
            let _lock = lock_for_server;
            let result = serve(listener, server_sender, cancelled).await;
            if let Err(error) = &result { tracing::error!(%error, "应用激活入口已停止"); }
            result
        });
        Ok(InstanceRole::Primary { instance: Self { lock: Some(lock), stop, server: Some(server) }, activations, sender })
    }
    pub async fn shutdown(&mut self) -> Result<()> {
        self.stop.cancel();
        let result = match self.server.take() { Some(server) => server.await.map_err(Error::from).and_then(|result| result), None => Ok(()) };
        // Never unlink the lock file. Another process may already hold an open
        // descriptor to its inode while waiting to become the next primary.
        self.lock = None; result
    }
}
impl Drop for SingleInstance { fn drop(&mut self) { self.stop.cancel(); } }

fn socket_name(directory: &Path) -> Result<Name<'static>> {
    let digest = format!("{:x}", Sha256::digest(directory.as_os_str().as_encoded_bytes()));
    let key = format!("super-paste-rs-{}", &digest[..32]);
    if GenericNamespaced::is_supported() { Ok(key.to_ns_name::<GenericNamespaced>()?) }
    else { Ok(std::env::temp_dir().join(format!("{key}.sock")).to_fs_name::<GenericFilePath>()?) }
}
fn valid_token(token: &str) -> bool { token.len() <= TOKEN_LIMIT && !token.contains('\0') }
fn authenticate(stream: &Stream) -> Result<()> {
    #[cfg(unix)] {
        let credentials = stream.peer_creds()?;
        if credentials.euid() != Some(rustix::process::geteuid().as_raw()) { return Err(Error::Platform("应用激活连接不属于当前用户".into())); }
    }
    #[cfg(windows)] { let _ = stream; /* Named pipes inherit the launching user's default DACL. */ }
    Ok(())
}
async fn serve(listener: Listener, sender: mpsc::Sender<Activation>, stop: CancellationToken) -> Result<()> {
    let mut clients = JoinSet::new();
    let result = loop {
        tokio::select! { biased;
            _ = stop.cancelled() => break Ok(()),
            completed = clients.join_next(), if !clients.is_empty() => {
                if let Some(Err(error)) = completed { tracing::warn!(%error, "应用激活连接任务异常退出"); }
            },
            accepted = listener.accept(), if clients.len() < 4 => {
                let stream = match accepted { Ok(stream) => stream, Err(error) if error.kind() == io::ErrorKind::Interrupted => continue, Err(error) => break Err(error.into()) };
                let sender = sender.clone(); let stop = stop.clone();
                clients.spawn(async move {
                    let result = tokio::time::timeout(IO_TIMEOUT, receive(stream, sender, stop)).await;
                    match result { Ok(Ok(())) => {}, Ok(Err(error)) => tracing::warn!(%error, "拒绝应用激活请求"), Err(_) => tracing::warn!("应用激活连接超时") }
                });
            }
        }
    };
    drop(listener);
    while let Some(completed) = clients.join_next().await { if let Err(error) = completed { tracing::warn!(%error, "应用激活连接回收失败"); } }
    result
}
async fn receive(mut stream: Stream, sender: mpsc::Sender<Activation>, stop: CancellationToken) -> Result<()> {
    authenticate(&stream)?;
    let mut header = [0u8; 8]; stream.read_exact(&mut header).await?;
    let length = u16::from_be_bytes([header[6], header[7]]) as usize;
    if &header[..5] != MAGIC || length > TOKEN_LIMIT || !matches!(header[5], 0..=2) || (header[5] != 1 && length != 0) { return Err(Error::Invalid("应用激活协议无效".into())); }
    let mut token = vec![0; length]; stream.read_exact(&mut token).await?;
    let token = if token.is_empty() { None } else { Some(String::from_utf8(token).map_err(|_| Error::Invalid("窗口激活令牌无效".into()))?) };
    if token.as_deref().is_some_and(|token| !valid_token(token)) { return Err(Error::Invalid("窗口激活令牌无效".into())); }
    let status = if stop.is_cancelled() { 2 } else if header[5] == 2 { 0 } else {
        let activation = if header[5] == 0 { Activation::ShowSettings } else { Activation::ToggleOverlay { activation_token: token } };
        match sender.try_send(activation) { Ok(()) => 0, Err(mpsc::error::TrySendError::Full(_)) => 1, Err(mpsc::error::TrySendError::Closed(_)) => 2 }
    };
    stream.write_all(&[MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], MAGIC[4], status]).await?;
    Ok(())
}
async fn forward(name: Name<'static>, request: LaunchRequest) -> Result<()> {
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut stream = loop {
        match tokio::time::timeout_at(deadline, Stream::connect(name.clone())).await {
            Ok(Ok(stream)) => break stream,
            Ok(Err(error)) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) && Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(25)).await,
            Ok(Err(error)) => return Err(Error::Platform(format!("无法连接已运行的应用：{error}").into())),
            Err(_) => return Err(Error::Platform("已运行的应用未能接收激活请求".into())),
        }
    };
    authenticate(&stream)?;
    let (command, token) = match request { LaunchRequest::Default => (0, None), LaunchRequest::ToggleOverlay { activation_token } => (1, activation_token), LaunchRequest::Background => (2, None) };
    let token = token.as_deref().unwrap_or("");
    if !valid_token(token) { return Err(Error::Invalid("窗口激活令牌无效".into())); }
    let length = (token.len() as u16).to_be_bytes();
    tokio::time::timeout(IO_TIMEOUT, async {
        stream.write_all(&[MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], MAGIC[4], command, length[0], length[1]]).await?;
        stream.write_all(token.as_bytes()).await?;
        let mut reply = [0; 6]; stream.read_exact(&mut reply).await?;
        if &reply[..5] != MAGIC { return Err(Error::Platform("已运行应用的激活协议不兼容".into())); }
        match reply[5] { 0 => Ok(()), 1 => Err(Error::ResourceLimit("应用激活请求过多，请稍后重试".into())), 2 => Err(Error::Platform("应用正在退出，无法接收激活请求".into())), _ => Err(Error::Platform("应用拒绝激活请求".into())) }
    }).await.map_err(|_| Error::Platform("应用激活确认超时".into()))?
}
