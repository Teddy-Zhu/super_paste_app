// Adapted from Super Paste src-tauri/src/platform/shortcut.rs; MIT, Teddy Zhu.
use std::{str::FromStr, sync::Arc};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use super::{PortalCommand, ShortcutSnapshot, ShortcutState, set_portal_state};

pub(super) async fn worker(
    mut commands: watch::Receiver<Option<PortalCommand>>,
    stop: CancellationToken,
    activate: Arc<dyn Fn(Option<String>) + Send + Sync>,
    state: watch::Sender<ShortcutSnapshot>,
) -> Result<(), String> {
    use ashpd::desktop::global_shortcuts::{
        Activated, BindShortcutsOptions, GlobalShortcuts, NewShortcut,
    };
    use ashpd::desktop::{CreateSessionOptions, Session};
    use futures_util::{Stream, StreamExt};
    use std::{future::pending, pin::Pin};

    type ActivatedStream = Pin<Box<dyn Stream<Item = Activated> + Send>>;

    enum BindOutcome {
        Stopped,
        Superseded(Option<PortalCommand>),
        Finished(Result<bool, String>),
    }

    let mut portal: Option<GlobalShortcuts> = None;
    let mut activated: Option<ActivatedStream> = None;
    let mut session: Option<Session<GlobalShortcuts>> = None;
    let mut pending_command: Option<PortalCommand> = None;

    loop {
        if let Some(command) = pending_command.take() {
            if stop.is_cancelled() {
                break;
            }

            if let Some(previous_session) = session.take() {
                if let Err(error) = close_portal_session(&previous_session).await {
                    portal = None;
                    activated = None;
                    set_portal_state(
                        &state,
                        &command,
                        ShortcutState::Failed,
                        Some(format!("关闭旧快捷键 portal 会话失败: {error}")),
                    );
                    continue;
                }
            }
            if stop.is_cancelled() {
                break;
            }

            if command.accelerator.is_empty() {
                set_portal_state(&state, &command, ShortcutState::Disabled, None);
                continue;
            }

            let trigger = match portal_trigger(&command.accelerator) {
                Ok(trigger) => trigger,
                Err(error) => {
                    set_portal_state(&state, &command, ShortcutState::Failed, Some(error));
                    continue;
                }
            };

            if portal.is_none() {
                match await_portal_operation(&mut commands, &stop, async {
                    GlobalShortcuts::new()
                        .await
                        .map_err(|error| error.to_string())
                })
                .await
                {
                    PortalWait::Stopped => break,
                    PortalWait::Superseded(next_command) => {
                        pending_command = next_command;
                        continue;
                    }
                    PortalWait::Finished(Ok(value)) => portal = Some(value),
                    PortalWait::Finished(Err(error)) => {
                        set_portal_state(
                            &state,
                            &command,
                            ShortcutState::Failed,
                            Some(format!("连接快捷键 portal 失败: {error}")),
                        );
                        continue;
                    }
                }
            }
            if !is_current(&state, &command) {
                pending_command = latest_command(&mut commands);
                continue;
            }

            if activated.is_none() {
                let Some(portal_proxy) = portal.as_ref() else {
                    set_portal_state(
                        &state,
                        &command,
                        ShortcutState::Failed,
                        Some("快捷键 portal 尚未初始化".to_string()),
                    );
                    continue;
                };
                match await_portal_operation(&mut commands, &stop, async {
                    portal_proxy
                        .receive_activated()
                        .await
                        .map_err(|error| error.to_string())
                })
                .await
                {
                    PortalWait::Stopped => break,
                    PortalWait::Superseded(next_command) => {
                        pending_command = next_command;
                        continue;
                    }
                    PortalWait::Finished(Ok(stream)) => activated = Some(Box::pin(stream)),
                    PortalWait::Finished(Err(error)) => {
                        set_portal_state(
                            &state,
                            &command,
                            ShortcutState::Failed,
                            Some(format!("订阅快捷键 portal 激活事件失败: {error}")),
                        );
                        continue;
                    }
                }
            }
            if stop.is_cancelled() {
                break;
            }
            if !is_current(&state, &command) {
                pending_command = latest_command(&mut commands);
                continue;
            }

            let create_result = {
                let Some(portal_proxy) = portal.as_ref() else {
                    set_portal_state(
                        &state,
                        &command,
                        ShortcutState::Failed,
                        Some("快捷键 portal 尚未初始化".to_string()),
                    );
                    continue;
                };
                await_portal_operation(&mut commands, &stop, async {
                    portal_proxy
                        .create_session(CreateSessionOptions::default())
                        .await
                        .map_err(|error| error.to_string())
                })
                .await
            };
            let next_session = match create_result {
                PortalWait::Stopped => {
                    // 会话请求可能已到达 portal；断开连接以释放无响应对应的远端会话。
                    drop(portal.take());
                    drop(activated.take());
                    break;
                }
                PortalWait::Superseded(next_command) => {
                    portal = None;
                    activated = None;
                    pending_command = next_command;
                    continue;
                }
                PortalWait::Finished(Ok(session)) => session,
                PortalWait::Finished(Err(error)) => {
                    set_portal_state(
                        &state,
                        &command,
                        ShortcutState::Failed,
                        Some(format!("创建快捷键 portal 会话失败: {error}")),
                    );
                    continue;
                }
            };

            if !is_current(&state, &command) {
                if let Err(error) = close_portal_session(&next_session).await {
                    tracing::error!("关闭过期快捷键 portal 会话失败: {error}");
                    portal = None;
                    activated = None;
                }
                pending_command = latest_command(&mut commands);
                continue;
            }

            let shortcut = NewShortcut::new("toggle-overlay", "Toggle overlay")
                .preferred_trigger(trigger.as_str());
            let bind_outcome = {
                let Some(portal_proxy) = portal.as_ref() else {
                    if let Err(error) = close_portal_session(&next_session).await {
                        tracing::error!("关闭快捷键 portal 会话失败: {error}");
                    }
                    set_portal_state(
                        &state,
                        &command,
                        ShortcutState::Failed,
                        Some("快捷键 portal 尚未初始化".to_string()),
                    );
                    continue;
                };
                let bind = async {
                    let request = portal_proxy
                        .bind_shortcuts(
                            &next_session,
                            &[shortcut],
                            None,
                            BindShortcutsOptions::default(),
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                    let binding = request.response().map_err(|error| error.to_string())?;
                    Ok::<bool, String>(
                        binding
                            .shortcuts()
                            .iter()
                            .any(|shortcut| shortcut.id() == "toggle-overlay"),
                    )
                };
                tokio::select! {
                    _ = stop.cancelled() => BindOutcome::Stopped,
                    changed = commands.changed() => {
                        if changed.is_err() {
                            BindOutcome::Stopped
                        } else {
                            BindOutcome::Superseded(latest_command(&mut commands))
                        }
                    }
                    result = bind => BindOutcome::Finished(result),
                }
            };

            match bind_outcome {
                BindOutcome::Stopped => {
                    if let Err(error) = close_portal_session(&next_session).await {
                        tracing::error!("关闭待授权快捷键 portal 会话失败: {error}");
                        drop(portal.take());
                        drop(activated.take());
                    }
                    break;
                }
                BindOutcome::Superseded(next_command) => {
                    // Closing the session also ends the portal dialog for this request.
                    if let Err(error) = close_portal_session(&next_session).await {
                        tracing::error!("取消旧快捷键 portal 请求失败: {error}");
                        portal = None;
                        activated = None;
                    }
                    pending_command = latest_command(&mut commands).or(next_command);
                }
                BindOutcome::Finished(Ok(true)) => {
                    if set_portal_state(&state, &command, ShortcutState::Registered, None) {
                        session = Some(next_session);
                    } else {
                        if let Err(error) = close_portal_session(&next_session).await {
                            tracing::error!("关闭过期快捷键 portal 会话失败: {error}");
                            portal = None;
                            activated = None;
                        }
                        pending_command = latest_command(&mut commands);
                    }
                }
                BindOutcome::Finished(Ok(false)) => {
                    let close_error = close_portal_session(&next_session).await.err();
                    if close_error.is_some() {
                        portal = None;
                        activated = None;
                    }
                    let error = close_error
                        .map(|error| format!("快捷键 portal 未授予授权；关闭会话失败: {error}"))
                        .unwrap_or_else(|| "快捷键 portal 未授予授权".to_string());
                    set_portal_state(&state, &command, ShortcutState::Failed, Some(error));
                }
                BindOutcome::Finished(Err(error)) => {
                    let close_error = close_portal_session(&next_session).await.err();
                    if close_error.is_some() {
                        portal = None;
                        activated = None;
                    }
                    let message = close_error
                        .map(|close| {
                            format!("快捷键 portal 请求失败: {error}; 关闭会话失败: {close}")
                        })
                        .unwrap_or_else(|| format!("快捷键 portal 请求失败: {error}"));
                    set_portal_state(&state, &command, ShortcutState::Failed, Some(message));
                }
            }
            continue;
        }

        let next_activated = async {
            match activated.as_mut() {
                Some(stream) => stream.as_mut().next().await,
                None => pending().await,
            }
        };

        tokio::select! {
            _ = stop.cancelled() => break,
            changed = commands.changed() => {
                if changed.is_err() {
                    break;
                }
                pending_command = latest_command(&mut commands);
            }
            event = next_activated => {
                match event {
                    Some(event) if event.shortcut_id() == "toggle-overlay" => {
                        let activation_token = event.options().get("activation_token")
                            .and_then(|value| String::try_from(value.clone()).ok());
                        activate_with_token(&activate, activation_token);
                    }
                    Some(_) => {}
                    None => {
                        activated = None;
                        if let Some(active_session) = session.take() {
                            let snapshot = state.borrow().clone();
                            let (generation, accelerator) = (snapshot.generation, snapshot.status.accelerator.to_string());
                            let command = PortalCommand { generation, accelerator };
                            let close_error = close_portal_session(&active_session).await.err();
                            if close_error.is_some() {
                                portal = None;
                            }
                            let error = close_error
                                .map(|error| format!("快捷键 portal 激活流已关闭；会话关闭失败: {error}"))
                                .unwrap_or_else(|| "快捷键 portal 激活流已关闭".to_string());
                            set_portal_state(&state, &command, ShortcutState::Failed, Some(error));
                        }
                    }
                }
            }
        }
    }

    if let Some(session) = session {
        if let Err(error) = close_portal_session(&session).await {
            return Err(format!("关闭快捷键 portal 会话失败：{error}"));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
enum PortalWait<T> {
    Stopped,
    Superseded(Option<PortalCommand>),
    Finished(Result<T, String>),
}

#[cfg(target_os = "linux")]
async fn await_portal_operation<T, F>(
    commands: &mut watch::Receiver<Option<PortalCommand>>,
    stop: &CancellationToken,
    operation: F,
) -> PortalWait<T>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    tokio::select! {
        _ = stop.cancelled() => PortalWait::Stopped,
        changed = commands.changed() => {
            if changed.is_err() {
                PortalWait::Stopped
            } else {
                PortalWait::Superseded(latest_command(commands))
            }
        }
        result = operation => PortalWait::Finished(result),
    }
}

#[cfg(target_os = "linux")]
async fn close_portal_session(
    session: &ashpd::desktop::Session<ashpd::desktop::global_shortcuts::GlobalShortcuts>,
) -> Result<(), String> {
    match tokio::time::timeout(std::time::Duration::from_secs(3), session.close()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("portal 会话关闭超时".to_string()),
    }
}

#[cfg(target_os = "linux")]
fn latest_command(commands: &mut watch::Receiver<Option<PortalCommand>>) -> Option<PortalCommand> {
    commands.borrow_and_update().clone()
}

#[cfg(target_os = "linux")]
fn is_current(state: &watch::Sender<ShortcutSnapshot>, command: &PortalCommand) -> bool {
    state.borrow().generation == command.generation
}

#[cfg(target_os = "linux")]
fn activate_with_token(
    activate: &Arc<dyn Fn(Option<String>) + Send + Sync>,
    activation_token: Option<String>,
) {
    activate(activation_token);
}

#[cfg(target_os = "linux")]
fn portal_trigger(accelerator: &str) -> Result<String, String> {
    let hotkey = global_hotkey::hotkey::HotKey::from_str(accelerator)
        .map_err(|error| format!("快捷键格式无效: {error}"))?;
    let key = hotkey.key;
    let mut parts = Vec::new();
    if hotkey
        .mods
        .contains(global_hotkey::hotkey::Modifiers::CONTROL)
    {
        parts.push("<Control>");
    }
    if hotkey.mods.contains(global_hotkey::hotkey::Modifiers::ALT) {
        parts.push("<Alt>");
    }
    if hotkey
        .mods
        .contains(global_hotkey::hotkey::Modifiers::SHIFT)
    {
        parts.push("<Shift>");
    }
    if hotkey
        .mods
        .contains(global_hotkey::hotkey::Modifiers::SUPER)
    {
        parts.push("<Super>");
    }
    let key_name = format!("{key:?}");
    let key_name = key_name
        .strip_prefix("Key")
        .or_else(|| key_name.strip_prefix("Digit"))
        .unwrap_or(&key_name);
    parts.push(key_name);
    Ok(parts.join(""))
}
