use super::EventWatcher;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(super) fn watch(source_cancelled: Arc<AtomicBool>) -> Result<EventWatcher, String> {
    let (event_sender, events) = mpsc::channel();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker_source_cancelled = source_cancelled.clone();
    let worker = thread::Builder::new()
        .name("x11-active-window-watch".into())
        .spawn(move || {
            let setup = initialize(&worker_source_cancelled);
            let (connection, root, active_window_atom, identity_atoms) = match setup {
                Ok(setup) => setup,
                Err(error) => {
                    let _ = ready_sender.send(Err(error));
                    return;
                }
            };
            if worker_source_cancelled.load(Ordering::Acquire)
                || worker_cancelled.load(Ordering::Acquire)
            {
                let _ = ready_sender.send(Err("X11 活动窗口监听已取消".into()));
                return;
            }
            let mut observed_window = None;
            observe_active_window(&connection, root, active_window_atom, &mut observed_window);
            if ready_sender.send(Ok(())).is_err() {
                return;
            }
            while !worker_source_cancelled.load(Ordering::Acquire)
                && !worker_cancelled.load(Ordering::Acquire)
            {
                match connection.poll_for_event() {
                    Ok(Some(xcb::Event::X(xcb::x::Event::PropertyNotify(event)))) => {
                        let activated =
                            event.window() == root && event.atom() == active_window_atom;
                        if activated {
                            observe_active_window(
                                &connection,
                                root,
                                active_window_atom,
                                &mut observed_window,
                            );
                        }
                        if (activated
                            || (Some(event.window()) == observed_window
                                && identity_atoms.contains(&event.atom())))
                            && event_sender.send(Ok(())).is_err()
                        {
                            return;
                        }
                    }
                    Ok(Some(xcb::Event::X(xcb::x::Event::DestroyNotify(event))))
                        if Some(event.window()) == observed_window =>
                    {
                        observed_window = None;
                        if event_sender.send(Ok(())).is_err() {
                            return;
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => thread::sleep(EVENT_POLL_INTERVAL),
                    Err(error) => {
                        let _ =
                            event_sender.send(Err(format!("读取 X11 活动窗口事件失败: {error:?}")));
                        return;
                    }
                }
            }
        })
        .map_err(|error| format!("无法启动 X11 活动窗口监听线程: {error}"))?;

    match ready_receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            cancelled.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(error);
        }
        Err(error) => {
            cancelled.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(format!("等待 X11 活动窗口监听初始化失败: {error}"));
        }
    }

    Ok(EventWatcher::new(events, move || {
        cancelled.store(true, Ordering::Release);
        let _ = worker.join();
    }))
}

fn initialize(
    cancelled: &AtomicBool,
) -> Result<
    (
        xcb::Connection,
        xcb::x::Window,
        xcb::x::Atom,
        Vec<xcb::x::Atom>,
    ),
    String,
> {
    use xcb::x;

    let (connection, screen_num) =
        xcb::Connection::connect(None).map_err(|error| format!("连接 X11 失败: {error:?}"))?;
    let screen = connection
        .get_setup()
        .roots()
        .nth(screen_num as usize)
        .ok_or_else(|| "X11 没有默认屏幕".to_owned())?;
    let root = screen.root();
    let atom_cookie = connection.send_request(&x::InternAtom {
        only_if_exists: true,
        name: b"_NET_ACTIVE_WINDOW",
    });
    let active_window_atom = connection
        .wait_for_reply(atom_cookie)
        .map_err(|error| format!("查询 X11 活动窗口属性失败: {error:?}"))?
        .atom();
    if active_window_atom == x::ATOM_NONE {
        return Err("X11 窗口管理器未提供 _NET_ACTIVE_WINDOW 属性".into());
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("X11 活动窗口监听已取消".into());
    }
    connection
        .send_and_check_request(&x::ChangeWindowAttributes {
            window: root,
            value_list: &[x::Cw::EventMask(x::EventMask::PROPERTY_CHANGE)],
        })
        .map_err(|error| format!("订阅 X11 根窗口属性事件失败: {error:?}"))?;
    connection
        .flush()
        .map_err(|error| format!("提交 X11 属性事件订阅失败: {error:?}"))?;
    let mut identity_atoms = vec![x::ATOM_WM_CLASS, x::ATOM_WM_TRANSIENT_FOR];
    for name in [b"_NET_WM_PID".as_slice(), b"WM_CLIENT_LEADER".as_slice()] {
        let atom = connection
            .wait_for_reply(connection.send_request(&x::InternAtom {
                only_if_exists: false,
                name,
            }))
            .map_err(|error| format!("读取 X11 应用身份属性失败: {error}"))?
            .atom();
        identity_atoms.push(atom);
    }
    Ok((connection, root, active_window_atom, identity_atoms))
}

// 有些临时窗口先激活、再设置 WM_CLASS/PID；仅监听根窗口会永久漏掉后续身份。
fn observe_active_window(
    connection: &xcb::Connection,
    root: xcb::x::Window,
    active_atom: xcb::x::Atom,
    observed: &mut Option<xcb::x::Window>,
) {
    use xcb::{Xid, x};
    let active = connection
        .wait_for_reply(connection.send_request(&x::GetProperty {
            delete: false,
            window: root,
            property: active_atom,
            r#type: x::ATOM_WINDOW,
            long_offset: 0,
            long_length: 1,
        }))
        .ok()
        .and_then(|reply| {
            (reply.format() == 32 && reply.r#type() == x::ATOM_WINDOW)
                .then(|| reply.value::<x::Window>().first().copied())
                .flatten()
        })
        .filter(|window| window.resource_id() > 1 && *window != root);
    if active == *observed {
        return;
    }
    if let Some(previous) = observed.take() {
        // 旧窗口可能已被销毁；取消本连接的订阅失败不影响新窗口。
        let _ = connection.send_and_check_request(&x::ChangeWindowAttributes {
            window: previous,
            value_list: &[x::Cw::EventMask(x::EventMask::NO_EVENT)],
        });
    }
    if let Some(window) = active {
        match connection.send_and_check_request(&x::ChangeWindowAttributes {
            window,
            value_list: &[x::Cw::EventMask(
                x::EventMask::PROPERTY_CHANGE | x::EventMask::STRUCTURE_NOTIFY,
            )],
        }) {
            Ok(()) => *observed = Some(window),
            Err(error) => tracing::debug!(%error, "活动窗口已消失或无法订阅身份属性"),
        }
    }
}
