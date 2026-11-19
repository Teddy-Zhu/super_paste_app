use std::collections::HashSet;
use xcb::{randr, x, Xid};
use super_paste_core::{Error, Result};
use crate::geometry::{Area, WorkAreaSnapshot, X11Root};

const MAX_CLIENTS: u32 = 4096;
const MAX_MONITORS: usize = 64;
struct Monitor { id: String, bounds: Rect, primary: bool }

#[derive(Clone, Copy, PartialEq)]
struct Rect { left: f64, top: f64, right: f64, bottom: f64 }
impl Rect {
    fn xywh(x: f64, y: f64, w: f64, h: f64) -> Self { Self { left: x, top: y, right: x + w, bottom: y + h } }
    fn intersect(self, other: Self) -> Option<Self> {
        let rect = Self { left: self.left.max(other.left), top: self.top.max(other.top), right: self.right.min(other.right), bottom: self.bottom.min(other.bottom) };
        (rect.left < rect.right && rect.top < rect.bottom).then_some(rect)
    }
    fn distance(self, x: f64, y: f64) -> f64 {
        (x - x.clamp(self.left, self.right)).powi(2) + (y - y.clamp(self.top, self.bottom)).powi(2)
    }
    fn contains(self, x: f64, y: f64) -> bool { x >= self.left && x < self.right && y >= self.top && y < self.bottom }
    fn area(self, display_id: String) -> Area { Area { display_id, x: self.left, y: self.top, width: self.right - self.left, height: self.bottom - self.top } }
}

pub(super) fn snapshot() -> Result<WorkAreaSnapshot> {
    let (connection, _) = xcb::Connection::connect_with_extensions(None, &[], &[xcb::Extension::RandR]).map_err(failure)?;
    if !connection.active_extensions().any(|extension| extension == xcb::Extension::RandR) { return Err(platform("X11 显示服务未提供 RandR 扩展")); }
    let version = connection.wait_for_reply(connection.send_request(&randr::QueryVersion { major_version: 1, minor_version: 5 })).map_err(failure)?;
    if (version.major_version(), version.minor_version()) < (1, 2) { return Err(platform("X11 需要 RandR 1.2 或更新版本")); }
    let mut cursor = None;
    for (index, screen) in connection.get_setup().roots().enumerate() {
        let pointer = connection.wait_for_reply(connection.send_request(&x::QueryPointer { window: screen.root() })).map_err(failure)?;
        if pointer.same_screen() { cursor = Some((index as u32, screen.root(), f64::from(pointer.root_x()), f64::from(pointer.root_y()))); break; }
    }
    let (index, root, x, y) = cursor.ok_or_else(|| platform("无法确定光标所在的 X11 屏幕"))?;
    let geometry = connection.wait_for_reply(connection.send_request(&x::GetGeometry { drawable: x::Drawable::Window(root) })).map_err(failure)?;
    let physical = Rect::xywh(0.0, 0.0, f64::from(geometry.width()), f64::from(geometry.height()));
    let mut monitors = Vec::new();
    if (version.major_version(), version.minor_version()) >= (1, 5) {
        let reply = connection.wait_for_reply(connection.send_request(&randr::GetMonitors { window: root, get_active: true })).map_err(failure)?;
        for monitor in reply.monitors() {
            if monitor.width() == 0 || monitor.height() == 0 { continue; }
            monitors.push(Monitor { id: format!("x11-{index}-{}", monitor.name().resource_id()), primary: monitor.primary(),
                bounds: Rect::xywh(f64::from(monitor.x()), f64::from(monitor.y()), f64::from(monitor.width()), f64::from(monitor.height())) });
            if monitors.len() > MAX_MONITORS { return Err(platform("X11 显示器数量超出限制")); }
        }
    } else {
        // RandR 1.2 has no logical MonitorInfo. Active CRTCs still provide
        // physical output geometry; cloned outputs share one usable rectangle.
        let primary = if (version.major_version(), version.minor_version()) >= (1, 3) {
            let output = connection.wait_for_reply(connection.send_request(&randr::GetOutputPrimary { window: root })).map_err(failure)?.output();
            (!output.is_none()).then_some(output)
        } else { None };
        let reply = connection.wait_for_reply(connection.send_request(&randr::GetScreenResources { window: root })).map_err(failure)?;
        if reply.crtcs().len() > MAX_MONITORS { return Err(platform("X11 输出数量超出限制")); }
        for crtc in reply.crtcs() {
            let info = connection.wait_for_reply(connection.send_request(&randr::GetCrtcInfo { crtc: *crtc, config_timestamp: reply.config_timestamp() })).map_err(failure)?;
            if info.status() != randr::SetConfig::Success { return Err(platform("X11 显示器布局已变化，请重试")); }
            if info.width() == 0 || info.height() == 0 { continue; }
            let rect = Rect::xywh(f64::from(info.x()), f64::from(info.y()), f64::from(info.width()), f64::from(info.height()));
            let is_primary = primary.is_some_and(|output| info.outputs().contains(&output));
            if let Some(existing) = monitors.iter_mut().find(|monitor| monitor.bounds == rect) { existing.primary |= is_primary; }
            else { monitors.push(Monitor { id: format!("x11-{index}-crtc-{}", crtc.resource_id()), primary: is_primary, bounds: rect }); }
        }
    }
    if monitors.is_empty() { return Err(platform("X11 没有可用的物理显示器")); }
    if !monitors.iter().any(|monitor| monitor.primary) { monitors[0].primary = true; }
    let selected = monitors.iter().position(|monitor| monitor.bounds.contains(x, y)).unwrap_or_else(||
        monitors.iter().enumerate().min_by(|(_, a), (_, b)| a.bounds.distance(x, y).total_cmp(&b.bounds.distance(x, y)))
            .map(|(index, _)| index).expect("nonempty monitor list"));
    let reader = Properties { connection: &connection };
    let desktop_atom = reader.atom(b"_NET_CURRENT_DESKTOP")?;
    let desktop = reader.property(root, desktop_atom, x::ATOM_CARDINAL, 1)?.as_ref().and_then(|reply| reply.value::<u32>().first().copied()).unwrap_or(0);
    let published = reader.monitor_work_areas(root, desktop)?;
    let struts = if published.is_none() { reader.struts(root, desktop, physical)? } else { Vec::new() };
    let fallback = if published.is_none() && struts.is_empty() { reader.work_area(root, desktop)? } else { None };
    let mut areas = Vec::with_capacity(monitors.len());
    for monitor in &monitors {
        let mut work = monitor.bounds;
        if let Some(published) = &published {
            // These rectangles are in root coordinates; their order need not
            // match RandR. A monitor omitted by the WM has no published inset.
            for area in published { if let Some(clipped) = work.intersect(*area) { work = clipped; } }
        } else {
            for strut in &struts { strut.apply(monitor.bounds, &mut work); }
            // A single desktop-wide EWMH rectangle cannot describe an L or
            // vertically stacked arrangement. Match the legacy host's primary
            // monitor fallback instead of cropping unrelated secondary screens.
            if monitor.primary { if let Some(fallback) = fallback { if let Some(clipped) = work.intersect(fallback) { work = clipped; } } }
        }
        let area = work.area(monitor.id.clone()); area.validate()?; areas.push(area);
    }
    let final_desktop = reader.property(root, desktop_atom, x::ATOM_CARDINAL, 1)?.as_ref().and_then(|reply| reply.value::<u32>().first().copied()).unwrap_or(0);
    let final_geometry = connection.wait_for_reply(connection.send_request(&x::GetGeometry { drawable: x::Drawable::Window(root) })).map_err(failure)?;
    if desktop != final_desktop || geometry.width() != final_geometry.width() || geometry.height() != final_geometry.height() {
        return Err(platform("X11 工作区已变化，请重新打开窗口"));
    }
    Ok(WorkAreaSnapshot { cursor_area: areas[selected].clone(), areas, x11_root: Some(X11Root { index, width: geometry.width(), height: geometry.height() }) })
}

struct Properties<'a> { connection: &'a xcb::Connection }
impl Properties<'_> {
    fn atom(&self, name: &[u8]) -> Result<x::Atom> {
        self.connection.wait_for_reply(self.connection.send_request(&x::InternAtom { only_if_exists: true, name })).map(|reply| reply.atom()).map_err(failure)
    }
    fn property(&self, window: x::Window, atom: x::Atom, kind: x::Atom, limit: u32) -> Result<Option<x::GetPropertyReply>> {
        let Some(cookie) = self.request(window, atom, kind, limit) else { return Ok(None); };
        Self::checked(self.reply(cookie)?, kind)
    }
    fn request(&self, window: x::Window, atom: x::Atom, kind: x::Atom, limit: u32) -> Option<x::GetPropertyCookie> {
        (!atom.is_none()).then(|| self.connection.send_request(&x::GetProperty { delete: false, window, property: atom, r#type: kind, long_offset: 0, long_length: limit }))
    }
    fn reply(&self, cookie: x::GetPropertyCookie) -> Result<Option<x::GetPropertyReply>> {
        match self.connection.wait_for_reply(cookie) {
            Ok(reply) => Ok(Some(reply)),
            // Clients can disappear between the root list and this request.
            Err(error) if gone(&error) => Ok(None),
            Err(error) => Err(failure(error)),
        }
    }
    fn checked(reply: Option<x::GetPropertyReply>, kind: x::Atom) -> Result<Option<x::GetPropertyReply>> {
        let Some(reply) = reply else { return Ok(None); };
        if reply.r#type().is_none() { return Ok(None); }
        if reply.format() != 32 || reply.r#type() != kind || reply.bytes_after() != 0 { return Err(platform("X11 工作区属性格式无效或超过限制")); }
        Ok(Some(reply))
    }
    fn struts(&self, root: x::Window, desktop: u32, screen: Rect) -> Result<Vec<Strut>> {
        let clients = self.property(root, self.atom(b"_NET_CLIENT_LIST")?, x::ATOM_WINDOW, MAX_CLIENTS)?;
        let Some(clients) = clients else { return Ok(Vec::new()); };
        let partial = self.atom(b"_NET_WM_STRUT_PARTIAL")?; let legacy = self.atom(b"_NET_WM_STRUT")?;
        let window_desktop = self.atom(b"_NET_WM_DESKTOP")?;
        let mut seen = HashSet::new(); let mut result = Vec::new();
        // Submit the bounded property batch before waiting. Ordinary app
        // windows have no struts; do not add two server round trips per client.
        let requests: Vec<_> = clients.value::<x::Window>().iter().copied().filter(|window| seen.insert(*window)).map(|window|
            (window, self.request(window, partial, x::ATOM_CARDINAL, 12), self.request(window, legacy, x::ATOM_CARDINAL, 4))).collect();
        for (window, partial, legacy) in requests {
            let partial = partial.map(|cookie| self.reply(cookie)).transpose()?.flatten();
            let legacy = legacy.map(|cookie| self.reply(cookie)).transpose()?.flatten();
            let values = if let Some(reply) = Self::checked(partial, x::ATOM_CARDINAL)? {
                let raw = reply.value::<u32>();
                if raw.len() != 12 { return Err(platform("X11 面板占位属性长度无效")); }
                let mut values = [0; 12]; values.copy_from_slice(raw); values
            } else if let Some(reply) = Self::checked(legacy, x::ATOM_CARDINAL)? {
                let raw = reply.value::<u32>();
                if raw.len() != 4 { return Err(platform("X11 面板占位属性长度无效")); }
                let mut values = [0; 12]; values[..4].copy_from_slice(raw);
                values[5] = (screen.bottom as u32).saturating_sub(1); values[7] = values[5];
                values[9] = (screen.right as u32).saturating_sub(1); values[11] = values[9]; values
            } else { continue; };
            if values[..4].iter().all(|value| *value == 0) { continue; }
            if self.property(window, window_desktop, x::ATOM_CARDINAL, 1)?.as_ref().and_then(|reply| reply.value::<u32>().first().copied())
                .is_some_and(|value| value != u32::MAX && value != desktop) { continue; }
            let attributes = match self.connection.wait_for_reply(self.connection.send_request(&x::GetWindowAttributes { window })) {
                Ok(reply) => reply,
                Err(error) if gone(&error) => continue,
                Err(error) => return Err(failure(error)),
            };
            if attributes.map_state() == x::MapState::Viewable { result.push(Strut { values, screen }); }
        }
        Ok(result)
    }
    fn work_area(&self, root: x::Window, desktop: u32) -> Result<Option<Rect>> {
        let Some(reply) = self.property(root, self.atom(b"_NET_WORKAREA")?, x::ATOM_CARDINAL, 4096)? else { return Ok(None); };
        let offset = usize::try_from(desktop).ok().and_then(|value| value.checked_mul(4)).ok_or_else(|| platform("X11 桌面编号超出范围"))?;
        let values = reply.value::<u32>();
        let raw = values.get(offset..offset.saturating_add(4)).ok_or_else(|| platform("X11 当前桌面工作区缺失"))?;
        if raw[2] == 0 || raw[3] == 0 { return Err(platform("X11 工作区尺寸无效")); }
        Ok(Some(Rect::xywh(f64::from(raw[0]), f64::from(raw[1]), f64::from(raw[2]), f64::from(raw[3]))))
    }
    fn monitor_work_areas(&self, root: x::Window, desktop: u32) -> Result<Option<Vec<Rect>>> {
        let feature = self.atom(b"_GTK_WORKAREAS")?;
        if feature.is_none() { return Ok(None); }
        let supported = self.property(root, self.atom(b"_NET_SUPPORTED")?, x::ATOM_ATOM, 4096)?;
        if !supported.as_ref().is_some_and(|reply| reply.value::<x::Atom>().contains(&feature)) { return Ok(None); }
        let property = self.atom(format!("_GTK_WORKAREAS_D{desktop}").as_bytes())?;
        let Some(reply) = self.property(root, property, x::ATOM_CARDINAL, MAX_MONITORS as u32 * 4)? else { return Ok(None); };
        let values = reply.value::<u32>();
        if values.len() % 4 != 0 { return Err(platform("X11 分屏工作区属性长度无效")); }
        let mut areas = Vec::with_capacity(values.len() / 4);
        for raw in values.chunks_exact(4) {
            if raw[2] == 0 || raw[3] == 0 { return Err(platform("X11 分屏工作区尺寸无效")); }
            areas.push(Rect::xywh(f64::from(raw[0]), f64::from(raw[1]), f64::from(raw[2]), f64::from(raw[3])));
        }
        Ok(Some(areas))
    }
}

// EWMH partial strut ranges are inclusive and relative to the entire root,
// not the target monitor. Intersect their ranges before trimming that monitor.
// https://specifications.freedesktop.org/wm/latest/ar01s05.html
struct Strut { values: [u32; 12], screen: Rect }
impl Strut {
    fn apply(&self, monitor: Rect, work: &mut Rect) {
        let v = self.values.map(f64::from); let screen = self.screen;
        let overlaps = |start: f64, end: f64, low: f64, high: f64| start <= end && start < high && end + 1.0 > low;
        // A strut extending through an entire neighboring monitor describes
        // the root-to-panel distance, not a usable inset for that neighbor.
        if v[0] > 0.0 && overlaps(v[4], v[5], monitor.top, monitor.bottom) && v[0] > monitor.left && v[0] < monitor.right { work.left = work.left.max(v[0].min(screen.right)); }
        if v[1] > 0.0 && overlaps(v[6], v[7], monitor.top, monitor.bottom) && screen.right - v[1] < monitor.right && screen.right - v[1] > monitor.left { work.right = work.right.min((screen.right - v[1]).max(screen.left)); }
        if v[2] > 0.0 && overlaps(v[8], v[9], monitor.left, monitor.right) && v[2] > monitor.top && v[2] < monitor.bottom { work.top = work.top.max(v[2].min(screen.bottom)); }
        if v[3] > 0.0 && overlaps(v[10], v[11], monitor.left, monitor.right) && screen.bottom - v[3] < monitor.bottom && screen.bottom - v[3] > monitor.top { work.bottom = work.bottom.min((screen.bottom - v[3]).max(screen.top)); }
    }
}
fn gone(error: &xcb::Error) -> bool { matches!(error, xcb::Error::Protocol(xcb::ProtocolError::X(x::Error::Window(_), _))) }
fn failure(error: impl std::fmt::Debug) -> Error { platform(format!("读取 X11 工作区失败：{error:?}")) }
fn platform(message: impl Into<std::sync::Arc<str>>) -> Error { Error::Platform(message.into()) }
