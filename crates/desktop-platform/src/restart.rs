//! The replacement process waits on its parent's pipe, consuming no polling
//! timer. The desktop authorizes it only after accepted work has drained; main
//! releases the pipe after final cleanup and single-instance ownership ends.
use std::{io::{Read, Write}, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use super_paste_core::{Error, Result};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, process::Child, sync::Mutex};

const ARGUMENT: &str = "--super-paste-rs-restart-handoff";
const READY: u8 = 0x52;
const COMMIT: u8 = 0x47;
#[derive(Default)]
pub struct Restart { state: Mutex<State> }
#[derive(Default)]
struct State { closed: bool, replacement: Option<Child> }
pub struct PreparedRestart { child: Option<Child> }
impl Restart {
    pub fn new() -> Arc<Self> { Arc::new(Self::default()) }
    pub async fn prepare(&self) -> Result<PreparedRestart> {
        if self.state.lock().await.closed { return Err(Error::Cancelled); }
        let mut command = tokio::process::Command::new(executable()?);
        command.args(std::env::args_os().skip(1).filter(|argument| argument != ARGUMENT)).arg(ARGUMENT)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        #[cfg(target_os = "windows")] { use std::os::windows::process::CommandExt; command.as_std_mut().creation_flags(0x08000000); }
        let child = command.spawn()?;
        let mut prepared = PreparedRestart { child: Some(child) };
        let child = prepared.child.as_mut().ok_or_else(protocol_error)?;
        let mut output = child.stdout.take().ok_or_else(protocol_error)?;
        let mut ready = [0];
        tokio::time::timeout(Duration::from_secs(10), output.read_exact(&mut ready)).await
            .map_err(|_| Error::Platform("新进程未能完成重启准备".into()))??;
        if ready[0] != READY { return Err(protocol_error()); }
        Ok(prepared)
    }
    pub async fn authorize(&self, mut prepared: PreparedRestart) -> Result<()> {
        let mut state = self.state.lock().await;
        if state.closed { return Err(Error::Cancelled); }
        if state.replacement.is_some() { return Err(Error::Conflict("已有重启操作等待退出".into())); }
        let input = prepared.child.as_mut().and_then(|child| child.stdin.as_mut()).ok_or_else(protocol_error)?;
        input.write_all(&[COMMIT]).await?;
        state.replacement = prepared.child.take();
        Ok(())
    }
    /// Called by main after cleanup, even when GPUI's native quit hook expires.
    pub async fn release_after_shutdown(&self) {
        let mut state = self.state.lock().await; state.closed = true;
        if let Some(mut replacement) = state.replacement.take() {
            drop(replacement.stdin.take());
            // Tokio's child handle does not kill on drop. The replacement is
            // now independent and can initialize its own native application.
        }
    }
}
impl Drop for PreparedRestart {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child { if let Err(error) = child.start_kill() { tracing::warn!(%error, "未授权的重启进程清理失败"); } }
    }
}
/// Runs before GPUI, SQLite, keychain access or observers are initialized.
/// EOF before authorization is cancellation, including a parent startup error.
pub fn accept_handoff() -> Result<bool> {
    if !std::env::args_os().skip(1).any(|argument| argument == ARGUMENT) { return Ok(true); }
    let mut output = std::io::stdout().lock(); output.write_all(&[READY])?; output.flush()?; drop(output);
    let mut input = std::io::stdin().lock(); let mut byte = [0];
    if input.read(&mut byte)? == 0 { return Ok(false); }
    if byte[0] != COMMIT { return Err(protocol_error()); }
    if input.read(&mut byte)? != 0 { return Err(protocol_error()); }
    drop(input);
    // The readiness pipe must not become the application's stdout: its reader
    // belongs to the exiting process. Re-exec with ordinary detached stdio.
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.args(std::env::args_os().skip(1).filter(|argument| argument != ARGUMENT))
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit());
    #[cfg(unix)] {
        use std::os::unix::process::CommandExt;
        // Preserve the PID that an AppImage launcher waits for, and therefore
        // its mount lifetime. No shell or external restart helper is needed.
        return Err(command.exec().into());
    }
    #[cfg(windows)] { command.spawn()?; Ok(false) }
}
fn executable() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    if let (Some(directory), Some(image)) = (std::env::var_os("APPDIR"), std::env::var_os("APPIMAGE")) {
        let image = PathBuf::from(image);
        if executable.starts_with(PathBuf::from(directory)) && image.is_file() { return Ok(image); }
    }
    Ok(executable)
}
fn protocol_error() -> Error { Error::Platform("重启进程握手失败".into()) }
