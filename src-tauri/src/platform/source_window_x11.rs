use super::active_window::ActiveWindow;
use std::{cell::RefCell, collections::VecDeque, path::Path};
use xcb::{Xid, x};

thread_local! {
    // 来源追踪按事件查询，复用连接，不读取无关的进程内存、窗口标题或几何信息。
    static RESOLVER: RefCell<Option<Resolver>> = const { RefCell::new(None) };
}

pub(super) fn current() -> Result<ActiveWindow, String> {
    RESOLVER.with(|cached| {
        let mut cached = cached.borrow_mut();
        if cached
            .as_ref()
            .is_some_and(|resolver| resolver.connection.has_error().is_err())
        {
            *cached = None;
        }
        if cached.is_none() {
            *cached = Some(Resolver::connect()?);
        }
        cached.as_ref().expect("initialized X11 resolver").current()
    })
}

struct Resolver {
    connection: xcb::Connection,
    root: x::Window,
    active_atom: x::Atom,
    pid_atom: x::Atom,
    leader_atom: x::Atom,
}

impl Resolver {
    fn connect() -> Result<Self, String> {
        let (connection, screen) = xcb::Connection::connect(None)
            .map_err(|error| format!("连接 X11 来源服务失败: {error}"))?;
        let root = connection
            .get_setup()
            .roots()
            .nth(screen as usize)
            .ok_or("X11 没有默认屏幕")?
            .root();
        let atom = |name: &[u8]| {
            connection
                .wait_for_reply(connection.send_request(&x::InternAtom {
                    only_if_exists: false,
                    name,
                }))
                .map(|reply| reply.atom())
                .map_err(|error| format!("读取 X11 来源属性失败: {error}"))
        };
        let active_atom = atom(b"_NET_ACTIVE_WINDOW")?;
        let pid_atom = atom(b"_NET_WM_PID")?;
        let leader_atom = atom(b"WM_CLIENT_LEADER")?;
        Ok(Self {
            connection,
            root,
            active_atom,
            pid_atom,
            leader_atom,
        })
    }

    fn current(&self) -> Result<ActiveWindow, String> {
        let active = self.window_property(self.root, self.active_atom);
        if let Some(window) = active.and_then(|window| self.resolve(window)) {
            return Ok(window);
        }
        // 截图和弹出窗口不一定受窗口管理器管理，实际输入焦点仍可指向应用窗口。
        let focus = self
            .connection
            .wait_for_reply(self.connection.send_request(&x::GetInputFocus {}))
            .map_err(|error| format!("读取 X11 输入焦点失败: {error}"))?
            .focus();
        if Some(focus) != active
            && let Some(window) = self.resolve(focus)
        {
            return Ok(window);
        }
        self.connection
            .has_error()
            .map_err(|error| format!("X11 来源连接失败: {error}"))?;
        Err("X11 活动窗口和输入焦点均没有可识别的应用".into())
    }

    fn resolve(&self, window: x::Window) -> Option<ActiveWindow> {
        let mut candidates = VecDeque::from([window]);
        let mut visited = Vec::new();
        let mut name = None;
        let mut path = None;
        let mut identity_window = window;
        // 仅沿窗口所属关系查找，禁止扫描任意客户端并猜测来源。
        while let Some(candidate) = candidates.pop_front() {
            if visited.len() >= 8 {
                break;
            }
            if candidate.resource_id() <= 1
                || candidate == self.root
                || visited.contains(&candidate)
            {
                continue;
            }
            visited.push(candidate);
            if name.is_none() {
                name = self.class_name(candidate);
                if name.is_some() {
                    identity_window = candidate;
                }
            }
            if path.is_none() {
                path = self
                    .cardinal_property(candidate, self.pid_atom)
                    .filter(|pid| *pid > 0)
                    .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok())
                    .map(|path| path.to_string_lossy().into_owned());
            }
            if name.is_some() && path.is_some() {
                break;
            }
            for atom in [x::ATOM_WM_TRANSIENT_FOR, self.leader_atom] {
                if let Some(related) = self.window_property(candidate, atom) {
                    candidates.push_back(related);
                }
            }
            if let Ok(tree) = self.connection.wait_for_reply(
                self.connection
                    .send_request(&x::QueryTree { window: candidate }),
            ) {
                candidates.push_back(tree.parent());
            }
        }
        // WM_CLASS 与 PID 独立：一个缺失不应抹掉另一个已经确认的应用身份。
        let name = name.or_else(|| path.as_deref().and_then(process_name))?;
        Some(ActiveWindow {
            id: identity_window.resource_id().to_string(),
            name,
            path,
            native: None,
        })
    }

    fn class_name(&self, window: x::Window) -> Option<String> {
        let reply = self
            .connection
            .wait_for_reply(self.connection.send_request(&x::GetProperty {
                delete: false,
                window,
                property: x::ATOM_WM_CLASS,
                r#type: x::ATOM_STRING,
                long_offset: 0,
                long_length: 1024,
            }))
            .ok()?;
        if reply.format() != 8 || reply.r#type() != x::ATOM_STRING {
            return None;
        }
        class_name(reply.value::<u8>())
    }

    fn cardinal_property(&self, window: x::Window, property: x::Atom) -> Option<u32> {
        let reply = self
            .connection
            .wait_for_reply(self.connection.send_request(&x::GetProperty {
                delete: false,
                window,
                property,
                r#type: x::ATOM_CARDINAL,
                long_offset: 0,
                long_length: 1,
            }))
            .ok()?;
        if reply.format() != 32 || reply.r#type() != x::ATOM_CARDINAL {
            return None;
        }
        reply.value::<u32>().first().copied()
    }

    fn window_property(&self, window: x::Window, property: x::Atom) -> Option<x::Window> {
        let reply = self
            .connection
            .wait_for_reply(self.connection.send_request(&x::GetProperty {
                delete: false,
                window,
                property,
                r#type: x::ATOM_WINDOW,
                long_offset: 0,
                long_length: 1,
            }))
            .ok()?;
        if reply.format() != 32 || reply.r#type() != x::ATOM_WINDOW {
            return None;
        }
        reply.value::<x::Window>().first().copied()
    }
}

fn class_name(bytes: &[u8]) -> Option<String> {
    let mut fields = bytes.split(|byte| *byte == 0).take(2).filter_map(|field| {
        std::str::from_utf8(field)
            .ok()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    });
    let instance = fields.next()?;
    Some(fields.next().unwrap_or(instance).to_owned())
}

fn process_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()?
        .to_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}
