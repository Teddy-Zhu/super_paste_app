use std::{path::PathBuf, sync::{Arc, Mutex}};
use super_paste_core::{Error, Result};
use tokio::runtime::Handle;
use crate::paths::AppPaths;
use crate::instance::BACKGROUND_ARGUMENT;

pub struct AutoStart { runtime: Handle, data: PathBuf }
pub struct AutoStartChange { runtime: Handle, state: Arc<Mutex<NativeChange>> }

impl AutoStart {
    pub fn new(runtime: Handle, paths: &AppPaths) -> Arc<Self> {
        Arc::new(Self { runtime, data: paths.data.clone() })
    }
    pub async fn prepare(&self, enabled: bool) -> Result<AutoStartChange> {
        let data = self.data.clone();
        let state = self.runtime.spawn_blocking(move || NativeChange::prepare(data, enabled)).await??;
        Ok(AutoStartChange { runtime: self.runtime.clone(), state: Arc::new(Mutex::new(state)) })
    }
    /// Reassert the persisted startup choice before configuration writers start.
    /// Native launch entries must also follow a moved or upgraded executable;
    /// their enabled flag alone cannot establish that the command is current.
    pub async fn reconcile(&self, enabled: bool) -> Result<()> {
        let data = self.data.clone();
        self.runtime.spawn_blocking(move || NativeChange::prepare(data, enabled)?.synchronize()).await?
    }
}
impl AutoStartChange {
    pub async fn apply(&mut self) -> Result<()> {
        let state = self.state.clone();
        self.runtime.spawn_blocking(move || state.lock().map_err(|_| Error::Task("自启状态锁损坏".into()))?.apply()).await?
    }
    pub async fn rollback(&mut self) -> Result<()> {
        let state = self.state.clone();
        self.runtime.spawn_blocking(move || state.lock().map_err(|_| Error::Task("自启状态锁损坏".into()))?.rollback()).await?
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NativeChange { entry: PathBuf, before: Option<Vec<u8>>, after: Option<Vec<u8>>, applied: bool }
#[cfg(target_os = "linux")]
impl NativeChange {
    fn prepare(data: PathBuf, enabled: bool) -> Result<Self> {
        let entry = dirs::config_dir().ok_or_else(|| Error::Platform("无法确定自启配置目录".into()))?
            .join("autostart").join(format!("{}.desktop", super_paste_format::APPLICATION_ID));
        let before = read_entry(&entry)?;
        let after = if enabled {
            let executable = crate::restart::launch_path()?;
            if !executable.is_absolute() || !executable.is_file() { return Err(Error::Platform("自启程序路径不可用".into())); }
            let executable = path_text(&executable)?;
            let icon = data.join("icons").join("autostart.png");
            let icon_text = path_text(&icon)?;
            let image = include_bytes!("../../../resources/app-icon.png");
            if read_entry(&icon)?.as_deref() != Some(image.as_slice()) { write_entry(&icon, Some(image))?; }
            let quoted = executable.replace('%', "%%").chars().fold(String::from("\""), |mut out, ch| {
                if matches!(ch, '"' | '\\' | '$' | '`') { out.push('\\'); }
                out.push(ch); out
            }) + "\"";
            Some(format!("[Desktop Entry]\nType=Application\nName={}\nExec={} {}\nIcon={}\nTerminal=false\nStartupWMClass={}\nX-GNOME-Autostart-enabled=true\n",
                super_paste_format::APPLICATION_NAME, desktop_value(&quoted), BACKGROUND_ARGUMENT, desktop_value(icon_text), super_paste_format::APPLICATION_ID).into_bytes())
        } else { None };
        Ok(Self { entry, before, after, applied: false })
    }
}
#[cfg(target_os = "macos")]
impl NativeChange {
    fn prepare(_: PathBuf, enabled: bool) -> Result<Self> {
        let entry = dirs::home_dir().ok_or_else(|| Error::Platform("无法确定自启配置目录".into()))?
            .join("Library").join("LaunchAgents").join(format!("{}.plist", super_paste_format::APPLICATION_ID));
        let before = read_entry(&entry)?;
        let after = if enabled {
            let executable = crate::restart::launch_path()?;
            if !executable.is_absolute() || !executable.is_file() { return Err(Error::Platform("自启程序路径不可用".into())); }
            let executable = executable.to_str().ok_or_else(|| Error::Platform("自启路径不是有效的 UTF-8".into()))?;
            let mut path = String::with_capacity(executable.len());
            for ch in executable.chars() {
                match ch {
                    '&' => path.push_str("&amp;"), '<' => path.push_str("&lt;"), '>' => path.push_str("&gt;"),
                    '"' => path.push_str("&quot;"), '\'' => path.push_str("&apos;"),
                    // Character references prevent XML newline normalization
                    // from changing a valid filesystem path.
                    '\n' => path.push_str("&#10;"), '\r' => path.push_str("&#13;"), '\t' => path.push_str("&#9;"),
                    ch if ch < '\u{20}' || matches!(ch, '\u{fffe}' | '\u{ffff}') => return Err(Error::Platform("自启路径包含 XML 无法表示的字符".into())),
                    ch => path.push(ch),
                }
            }
            Some(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                <plist version=\"1.0\"><dict>\n\
                <key>Label</key><string>{}</string>\n\
                <key>ProgramArguments</key><array><string>{path}</string><string>{BACKGROUND_ARGUMENT}</string></array>\n\
                <key>RunAtLoad</key><true/>\n\
                </dict></plist>\n", super_paste_format::APPLICATION_ID).into_bytes())
        } else { None };
        Ok(Self { entry, before, after, applied: false })
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl NativeChange {
    fn synchronize(&mut self) -> Result<()> { self.apply() }
    fn apply(&mut self) -> Result<()> {
        if read_entry(&self.entry)? != self.before { return Err(Error::Conflict("自启条目已被其他操作修改".into())); }
        if self.before == self.after { return Ok(()); }
        write_entry(&self.entry, self.after.as_deref())?;
        self.applied = true;
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        if !self.applied { return Ok(()); }
        if read_entry(&self.entry)? != self.after { return Err(Error::Conflict("自启条目已被系统修改，无法回滚".into())); }
        write_entry(&self.entry, self.before.as_deref())?;
        self.applied = false;
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn path_text(path: &std::path::Path) -> Result<&str> {
    path.to_str().filter(|value| !value.chars().any(char::is_control))
        .ok_or_else(|| Error::Platform("自启路径不是有效的桌面条目路径".into()))
}
#[cfg(target_os = "linux")]
fn desktop_value(value: &str) -> String { value.replace('\\', "\\\\") }
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_entry(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 { return Err(Error::ResourceLimit("自启资源文件过大".into())); }
    Ok(Some(bytes))
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_entry(path: &std::path::Path, bytes: Option<&[u8]>) -> Result<()> {
    use std::io::Write;
    match bytes {
        Some(bytes) => {
            let directory = path.parent().ok_or_else(|| Error::Platform("自启条目目录无效".into()))?;
            std::fs::create_dir_all(directory)?;
            let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
            temporary.write_all(bytes)?;
            temporary.as_file().sync_all()?;
            temporary.persist(path).map_err(|error| Error::from(error.error))?;
            Ok(())
        }
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

#[cfg(target_os = "windows")]
struct NativeChange { launcher: auto_launch::AutoLaunch, before: bool, after: bool, applied: bool }
#[cfg(target_os = "windows")]
impl NativeChange {
    fn synchronize(&mut self) -> Result<()> {
        if self.after { self.launcher.enable() } else { self.launcher.disable() }.map_err(native_error)
    }
    fn prepare(_: PathBuf, after: bool) -> Result<Self> {
        use auto_launch::AutoLaunchBuilder;
        let executable = crate::restart::launch_path()?;
        let path = executable.to_str().ok_or_else(|| Error::Platform("自启路径不是有效的 UTF-8".into()))?;
        let mut builder = AutoLaunchBuilder::new();
        // auto-launch 0.6 appends arguments directly to its path. Registry
        // startup commands must retain paths containing spaces as one token.
        builder.set_app_name(super_paste_format::APPLICATION_ID).set_app_path(&format!("\"{path}\""))
            .set_args(&[BACKGROUND_ARGUMENT]).set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
        let launcher = builder.build().map_err(native_error)?;
        let before = launcher.is_enabled().map_err(native_error)?;
        Ok(Self { launcher, before, after, applied: false })
    }
    fn apply(&mut self) -> Result<()> {
        if self.launcher.is_enabled().map_err(native_error)? != self.before { return Err(Error::Conflict("自启条目已被其他操作修改".into())); }
        if self.before == self.after { return Ok(()); }
        // Native APIs may report failure after changing the entry; rollback inspects it.
        self.applied = true;
        if self.after { self.launcher.enable() } else { self.launcher.disable() }.map_err(native_error)
    }
    fn rollback(&mut self) -> Result<()> {
        if !self.applied { return Ok(()); }
        let current = self.launcher.is_enabled().map_err(native_error)?;
        if current == self.before { self.applied = false; return Ok(()); }
        if self.before { self.launcher.enable() } else { self.launcher.disable() }.map_err(native_error)?;
        self.applied = false;
        Ok(())
    }
}
#[cfg(target_os = "windows")]
fn native_error(error: impl std::fmt::Display) -> Error { Error::Platform(format!("自启操作失败：{error}").into()) }
