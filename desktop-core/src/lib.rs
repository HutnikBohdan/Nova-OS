#![no_std]

//! Allocation-free desktop/compositor policy for Nova OS.
//! The crate deliberately contains no framebuffer, host or kernel APIs: the kernel
//! supplies pixels and events while this module owns deterministic desktop state.

pub const MAX_WINDOWS: usize = 32;
pub const MAX_DAMAGE_RECTS: usize = 64;
pub const MAX_APPS: usize = 32;
pub const MAX_NOTIFICATIONS: usize = 24;
pub const MAX_TEXT_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub const fn right(self) -> i64 {
        self.x as i64 + self.width as i64
    }
    pub const fn bottom(self) -> i64 {
        self.y as i64 + self.height as i64
    }
    pub fn contains(self, p: Point) -> bool {
        p.x as i64 >= self.x as i64
            && p.y as i64 >= self.y as i64
            && (p.x as i64) < self.right()
            && (p.y as i64) < self.bottom()
    }
    pub fn intersects(self, other: Self) -> bool {
        (self.x as i64) < other.right()
            && (other.x as i64) < self.right()
            && (self.y as i64) < other.bottom()
            && (other.y as i64) < self.bottom()
    }
    pub fn union(self, other: Self) -> Self {
        let left = (self.x as i64).min(other.x as i64);
        let top = (self.y as i64).min(other.y as i64);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        )
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = (self.x as i64).max(other.x as i64);
        let top = (self.y as i64).max(other.y as i64);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (left < right && top < bottom).then(|| {
            Self::new(
                left as i32,
                top as i32,
                (right - left) as u32,
                (bottom - top) as u32,
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextError {
    TooLong,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FixedText {
    bytes: [u8; MAX_TEXT_BYTES],
    len: u8,
}

impl FixedText {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; MAX_TEXT_BYTES],
            len: 0,
        }
    }
    pub fn new(value: &str) -> Result<Self, TextError> {
        if value.len() > MAX_TEXT_BYTES {
            return Err(TextError::TooLong);
        }
        let mut text = Self::empty();
        text.bytes[..value.len()].copy_from_slice(value.as_bytes());
        text.len = value.len() as u8;
        Ok(text)
    }
    pub fn as_str(&self) -> &str {
        // Only `new` can populate the buffer and it accepts a valid `str`.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }
}

impl core::fmt::Debug for FixedText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_str().fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibleRole {
    Desktop,
    Window,
    Dialog,
    Button,
    TextField,
    Document,
    Notification,
}

impl AccessibleRole {
    pub const fn ukrainian_name(self) -> &'static str {
        match self {
            Self::Desktop => "Робочий стіл",
            Self::Window => "Вікно",
            Self::Dialog => "Діалогове вікно",
            Self::Button => "Кнопка",
            Self::TextField => "Текстове поле",
            Self::Document => "Документ",
            Self::Notification => "Сповіщення",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accessibility {
    pub role: AccessibleRole,
    pub label: FixedText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub app: AppId,
    pub bounds: Rect,
    pub restore_bounds: Rect,
    pub min_size: Size,
    pub max_size: Size,
    pub state: WindowState,
    pub focusable: bool,
    pub accessibility: Accessibility,
}

impl Window {
    pub fn new(id: WindowId, app: AppId, bounds: Rect, label: &str) -> Result<Self, TextError> {
        Ok(Self {
            id,
            app,
            bounds,
            restore_bounds: bounds,
            min_size: Size {
                width: 160,
                height: 96,
            },
            max_size: Size {
                width: u32::MAX,
                height: u32::MAX,
            },
            state: WindowState::Normal,
            focusable: true,
            accessibility: Accessibility {
                role: AccessibleRole::Window,
                label: FixedText::new(label)?,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopError {
    Full,
    DuplicateId,
    UnknownWindow,
    InvalidWorkspace,
    InvalidConstraint,
}

pub struct DamageTracker {
    rects: [Rect; MAX_DAMAGE_RECTS],
    len: usize,
    full: Option<Rect>,
}

impl DamageTracker {
    pub const fn new() -> Self {
        Self {
            rects: [Rect::new(0, 0, 0, 0); MAX_DAMAGE_RECTS],
            len: 0,
            full: None,
        }
    }
    pub fn clear(&mut self) {
        self.len = 0;
        self.full = None;
    }
    pub fn rects(&self) -> &[Rect] {
        if let Some(ref full) = self.full {
            core::slice::from_ref(full)
        } else {
            &self.rects[..self.len]
        }
    }
    pub fn add(&mut self, rect: Rect, screen: Rect) {
        let Some(mut clipped) = rect.intersection(screen) else {
            return;
        };
        if self.full.is_some() {
            return;
        }
        let mut index = 0;
        while index < self.len {
            // Adjacent/overlapping rectangles are coalesced to keep repaint bounded.
            let expanded = Rect::new(
                clipped.x.saturating_sub(1),
                clipped.y.saturating_sub(1),
                clipped.width.saturating_add(2),
                clipped.height.saturating_add(2),
            );
            if expanded.intersects(self.rects[index]) {
                clipped = clipped.union(self.rects[index]);
                self.len -= 1;
                self.rects[index] = self.rects[self.len];
            } else {
                index += 1;
            }
        }
        if self.len == MAX_DAMAGE_RECTS {
            self.len = 0;
            self.full = Some(screen);
        } else {
            self.rects[self.len] = clipped;
            self.len += 1;
        }
    }
}

impl Default for DamageTracker {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SceneGraph {
    screen: Rect,
    windows: [Option<Window>; MAX_WINDOWS],
    z_order: [Option<WindowId>; MAX_WINDOWS],
    len: usize,
    focused: Option<WindowId>,
    pub damage: DamageTracker,
}

impl SceneGraph {
    pub const fn new(screen: Rect) -> Self {
        Self {
            screen,
            windows: [None; MAX_WINDOWS],
            z_order: [None; MAX_WINDOWS],
            len: 0,
            focused: None,
            damage: DamageTracker::new(),
        }
    }
    pub const fn focused(&self) -> Option<WindowId> {
        self.focused
    }
    pub const fn window_count(&self) -> usize {
        self.len
    }
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().flatten().find(|window| window.id == id)
    }
    fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows
            .iter_mut()
            .flatten()
            .find(|window| window.id == id)
    }
    pub fn z_order(&self) -> impl DoubleEndedIterator<Item = WindowId> + '_ {
        self.z_order[..self.len].iter().filter_map(|id| *id)
    }
    pub fn add_window(&mut self, window: Window) -> Result<(), DesktopError> {
        if self.window(window.id).is_some() {
            return Err(DesktopError::DuplicateId);
        }
        if self.len == MAX_WINDOWS {
            return Err(DesktopError::Full);
        }
        let slot = self
            .windows
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(DesktopError::Full)?;
        let id = window.id;
        let bounds = window.bounds;
        *slot = Some(window);
        self.z_order[self.len] = Some(id);
        self.len += 1;
        self.damage.add(bounds, self.screen);
        if window.focusable {
            self.focus(id)?;
        }
        Ok(())
    }
    pub fn focus(&mut self, id: WindowId) -> Result<(), DesktopError> {
        let (focusable, state, bounds) = self
            .window(id)
            .map(|window| (window.focusable, window.state, window.bounds))
            .ok_or(DesktopError::UnknownWindow)?;
        if !focusable || matches!(state, WindowState::Minimized | WindowState::Closed) {
            return Ok(());
        }
        let position = self.z_order[..self.len]
            .iter()
            .position(|entry| *entry == Some(id))
            .ok_or(DesktopError::UnknownWindow)?;
        self.z_order.copy_within(position + 1..self.len, position);
        self.z_order[self.len - 1] = Some(id);
        self.focused = Some(id);
        self.damage.add(bounds, self.screen);
        Ok(())
    }
    pub fn hit_test(&self, point: Point) -> Option<WindowId> {
        self.z_order().rev().find(|id| {
            self.window(*id).is_some_and(|window| {
                !matches!(window.state, WindowState::Minimized | WindowState::Closed)
                    && window.bounds.contains(point)
            })
        })
    }
    pub fn move_window(&mut self, id: WindowId, origin: Point) -> Result<(), DesktopError> {
        let screen = self.screen;
        let old = self.window(id).ok_or(DesktopError::UnknownWindow)?.bounds;
        let max_x = screen
            .right()
            .saturating_sub(old.width as i64)
            .max(screen.x as i64);
        let max_y = screen
            .bottom()
            .saturating_sub(old.height as i64)
            .max(screen.y as i64);
        let x = (origin.x as i64).clamp(screen.x as i64, max_x) as i32;
        let y = (origin.y as i64).clamp(screen.y as i64, max_y) as i32;
        let new = {
            let window = self.window_mut(id).unwrap();
            window.bounds.x = x;
            window.bounds.y = y;
            window.restore_bounds = window.bounds;
            window.bounds
        };
        self.damage.add(old, screen);
        self.damage.add(new, screen);
        Ok(())
    }
    pub fn resize_window(&mut self, id: WindowId, requested: Size) -> Result<(), DesktopError> {
        let screen = self.screen;
        let window = self.window(id).ok_or(DesktopError::UnknownWindow)?;
        if window.min_size.width > window.max_size.width
            || window.min_size.height > window.max_size.height
        {
            return Err(DesktopError::InvalidConstraint);
        }
        let old = window.bounds;
        let available_width = screen.right().saturating_sub(old.x as i64).max(1) as u32;
        let available_height = screen.bottom().saturating_sub(old.y as i64).max(1) as u32;
        let width = requested
            .width
            .clamp(window.min_size.width, window.max_size.width)
            .min(available_width);
        let height = requested
            .height
            .clamp(window.min_size.height, window.max_size.height)
            .min(available_height);
        let new = {
            let window = self.window_mut(id).unwrap();
            window.bounds.width = width;
            window.bounds.height = height;
            window.restore_bounds = window.bounds;
            window.bounds
        };
        self.damage.add(old, screen);
        self.damage.add(new, screen);
        Ok(())
    }
    pub fn minimize(&mut self, id: WindowId) -> Result<(), DesktopError> {
        let screen = self.screen;
        let window = self.window_mut(id).ok_or(DesktopError::UnknownWindow)?;
        let old = window.bounds;
        window.state = WindowState::Minimized;
        if self.focused == Some(id) {
            self.focused = None;
        }
        self.damage.add(old, screen);
        Ok(())
    }
    pub fn maximize(&mut self, id: WindowId) -> Result<(), DesktopError> {
        let screen = self.screen;
        let window = self.window_mut(id).ok_or(DesktopError::UnknownWindow)?;
        let old = window.bounds;
        if window.state != WindowState::Maximized {
            window.restore_bounds = old;
        }
        window.bounds = screen;
        window.state = WindowState::Maximized;
        self.damage.add(old, screen);
        self.damage.add(screen, screen);
        Ok(())
    }
    pub fn restore(&mut self, id: WindowId) -> Result<(), DesktopError> {
        let screen = self.screen;
        let (old, new, focusable) = {
            let window = self.window_mut(id).ok_or(DesktopError::UnknownWindow)?;
            let old = window.bounds;
            window.bounds = window.restore_bounds;
            window.state = WindowState::Normal;
            (old, window.bounds, window.focusable)
        };
        self.damage.add(old, screen);
        self.damage.add(new, screen);
        if focusable {
            self.focus(id)?;
        }
        Ok(())
    }
    pub fn close(&mut self, id: WindowId) -> Result<Window, DesktopError> {
        let index = self
            .windows
            .iter()
            .position(|slot| slot.is_some_and(|w| w.id == id))
            .ok_or(DesktopError::UnknownWindow)?;
        let mut removed = self.windows[index].take().unwrap();
        removed.state = WindowState::Closed;
        let z = self.z_order[..self.len]
            .iter()
            .position(|entry| *entry == Some(id))
            .unwrap();
        self.z_order.copy_within(z + 1..self.len, z);
        self.len -= 1;
        self.z_order[self.len] = None;
        if self.focused == Some(id) {
            self.focused = self.z_order[..self.len]
                .iter()
                .rev()
                .filter_map(|v| *v)
                .find(|candidate| {
                    self.window(*candidate)
                        .is_some_and(|w| w.focusable && w.state == WindowState::Normal)
                });
        }
        self.damage.add(removed.bounds, self.screen);
        Ok(removed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Character(char),
    Enter,
    Escape,
    Tab,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    PointerDown(Point),
    PointerMove(Point),
    PointerUp(Point),
    KeyDown(Key),
    KeyUp(Key),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTarget {
    Window(WindowId),
    Desktop,
    GlobalShortcut,
}

pub struct InputRouter {
    capture: Option<WindowId>,
}

impl InputRouter {
    pub const fn new() -> Self {
        Self { capture: None }
    }
    pub const fn captured(&self) -> Option<WindowId> {
        self.capture
    }
    pub fn release_capture(&mut self) {
        self.capture = None;
    }
    pub fn route(&mut self, scene: &mut SceneGraph, event: InputEvent) -> InputTarget {
        match event {
            InputEvent::PointerDown(point) => {
                if let Some(id) = scene.hit_test(point) {
                    let _ = scene.focus(id);
                    self.capture = Some(id);
                    InputTarget::Window(id)
                } else {
                    InputTarget::Desktop
                }
            }
            InputEvent::PointerMove(point) => self
                .capture
                .map(InputTarget::Window)
                .or_else(|| scene.hit_test(point).map(InputTarget::Window))
                .unwrap_or(InputTarget::Desktop),
            InputEvent::PointerUp(point) => {
                let target = self.capture.take().or_else(|| scene.hit_test(point));
                target
                    .map(InputTarget::Window)
                    .unwrap_or(InputTarget::Desktop)
            }
            InputEvent::KeyDown(Key::Tab) => InputTarget::GlobalShortcut,
            InputEvent::KeyDown(_) | InputEvent::KeyUp(_) => scene
                .focused()
                .map(InputTarget::Window)
                .unwrap_or(InputTarget::Desktop),
        }
    }
}

impl Default for InputRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Starting,
    Running,
    Suspended,
    Stopped,
    Crashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppSession {
    pub app: AppId,
    pub state: AppState,
    pub active_window: Option<WindowId>,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSnapshot {
    entries: [Option<AppSession>; MAX_APPS],
    len: usize,
}

impl SessionSnapshot {
    pub const fn len(&self) -> usize {
        self.len
    }
    pub fn iter(&self) -> impl Iterator<Item = AppSession> + '_ {
        self.entries[..self.len].iter().filter_map(|v| *v)
    }
}

pub struct AppLifecycle {
    entries: [Option<AppSession>; MAX_APPS],
    len: usize,
}

impl AppLifecycle {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_APPS],
            len: 0,
        }
    }
    pub fn session(&self, app: AppId) -> Option<&AppSession> {
        self.entries.iter().flatten().find(|entry| entry.app == app)
    }
    pub fn launch(&mut self, app: AppId) -> Result<(), DesktopError> {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.app == app)
        {
            entry.state = AppState::Running;
            entry.revision = entry.revision.wrapping_add(1);
            return Ok(());
        }
        if self.len == MAX_APPS {
            return Err(DesktopError::Full);
        }
        self.entries[self.len] = Some(AppSession {
            app,
            state: AppState::Running,
            active_window: None,
            revision: 1,
        });
        self.len += 1;
        Ok(())
    }
    pub fn transition(&mut self, app: AppId, state: AppState) -> Result<(), DesktopError> {
        let entry = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.app == app)
            .ok_or(DesktopError::UnknownWindow)?;
        entry.state = state;
        entry.revision = entry.revision.wrapping_add(1);
        Ok(())
    }
    pub fn set_active_window(
        &mut self,
        app: AppId,
        window: Option<WindowId>,
    ) -> Result<(), DesktopError> {
        let entry = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.app == app)
            .ok_or(DesktopError::UnknownWindow)?;
        entry.active_window = window;
        entry.revision = entry.revision.wrapping_add(1);
        Ok(())
    }
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            entries: self.entries,
            len: self.len,
        }
    }
    pub fn restore(&mut self, snapshot: &SessionSnapshot) {
        self.entries = snapshot.entries;
        self.len = snapshot.len.min(MAX_APPS);
        for entry in self.entries[..self.len].iter_mut().flatten() {
            // A restored application must pass through startup, never masquerade as live.
            if matches!(entry.state, AppState::Running | AppState::Starting) {
                entry.state = AppState::Starting;
            }
        }
    }
}

impl Default for AppLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationUrgency {
    Low,
    Normal,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notification {
    pub id: u64,
    pub source: AppId,
    pub title: FixedText,
    pub body: FixedText,
    pub urgency: NotificationUrgency,
    pub expires_at_ms: Option<u64>,
}

impl Notification {
    pub fn new(
        id: u64,
        source: AppId,
        title: &str,
        body: &str,
        urgency: NotificationUrgency,
        expires_at_ms: Option<u64>,
    ) -> Result<Self, TextError> {
        Ok(Self {
            id,
            source,
            title: FixedText::new(title)?,
            body: FixedText::new(body)?,
            urgency,
            expires_at_ms,
        })
    }
}

pub struct NotificationCenter {
    entries: [Option<Notification>; MAX_NOTIFICATIONS],
    head: usize,
    len: usize,
}

impl NotificationCenter {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_NOTIFICATIONS],
            head: 0,
            len: 0,
        }
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub fn push(&mut self, notification: Notification) {
        if let Some(index) = (0..self.len).find(|offset| {
            self.entries[(self.head + offset) % MAX_NOTIFICATIONS]
                .is_some_and(|entry| entry.id == notification.id)
        }) {
            self.entries[(self.head + index) % MAX_NOTIFICATIONS] = Some(notification);
            return;
        }
        if self.len == MAX_NOTIFICATIONS {
            self.entries[self.head] = Some(notification);
            self.head = (self.head + 1) % MAX_NOTIFICATIONS;
        } else {
            let tail = (self.head + self.len) % MAX_NOTIFICATIONS;
            self.entries[tail] = Some(notification);
            self.len += 1;
        }
    }
    pub fn get(&self, logical_index: usize) -> Option<&Notification> {
        (logical_index < self.len)
            .then(|| self.entries[(self.head + logical_index) % MAX_NOTIFICATIONS].as_ref())
            .flatten()
    }
    pub fn dismiss(&mut self, id: u64) -> bool {
        let Some(offset) =
            (0..self.len).find(|offset| self.get(*offset).is_some_and(|n| n.id == id))
        else {
            return false;
        };
        for index in offset..self.len - 1 {
            let next = (self.head + index + 1) % MAX_NOTIFICATIONS;
            let current = (self.head + index) % MAX_NOTIFICATIONS;
            self.entries[current] = self.entries[next];
        }
        let tail = (self.head + self.len - 1) % MAX_NOTIFICATIONS;
        self.entries[tail] = None;
        self.len -= 1;
        true
    }
    pub fn expire(&mut self, now_ms: u64) {
        let mut index = 0;
        while index < self.len {
            let expired = self
                .get(index)
                .and_then(|n| n.expires_at_ms)
                .is_some_and(|deadline| deadline <= now_ms);
            if expired {
                let id = self.get(index).unwrap().id;
                self.dismiss(id);
            } else {
                index += 1;
            }
        }
    }
}

impl Default for NotificationCenter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    fn window(id: u32, x: i32, y: i32, w: u32, h: u32) -> Window {
        Window::new(
            WindowId(id),
            AppId(id),
            Rect::new(x, y, w, h),
            "Тестове вікно",
        )
        .unwrap()
    }

    #[test]
    fn ukrainian_accessibility_is_utf8_safe() {
        let text = FixedText::new("Редактор документів").unwrap();
        assert_eq!(text.as_str(), "Редактор документів");
        assert_eq!(AccessibleRole::Notification.ukrainian_name(), "Сповіщення");
    }
    #[test]
    fn rect_geometry_uses_exclusive_edges() {
        let rect = Rect::new(10, 20, 100, 50);
        assert!(rect.contains(Point { x: 109, y: 69 }));
        assert!(!rect.contains(Point { x: 110, y: 69 }));
        assert_eq!(
            rect.intersection(Rect::new(90, 60, 30, 30)),
            Some(Rect::new(90, 60, 20, 10))
        );
    }
    #[test]
    fn focus_raises_window_and_hit_test_obeys_z_order() {
        let mut scene = SceneGraph::new(Rect::new(0, 0, 800, 600));
        scene.add_window(window(1, 0, 0, 300, 300)).unwrap();
        scene.add_window(window(2, 100, 100, 300, 300)).unwrap();
        assert_eq!(scene.hit_test(Point { x: 150, y: 150 }), Some(WindowId(2)));
        scene.focus(WindowId(1)).unwrap();
        assert_eq!(scene.hit_test(Point { x: 150, y: 150 }), Some(WindowId(1)));
    }
    #[test]
    fn duplicate_window_is_rejected() {
        let mut scene = SceneGraph::new(Rect::new(0, 0, 800, 600));
        scene.add_window(window(7, 0, 0, 100, 100)).unwrap();
        assert_eq!(
            scene.add_window(window(7, 1, 1, 100, 100)),
            Err(DesktopError::DuplicateId)
        );
    }
    #[test]
    fn move_and_resize_are_constrained_to_workspace() {
        let mut scene = SceneGraph::new(Rect::new(0, 0, 800, 600));
        let mut w = window(1, 10, 10, 300, 200);
        w.min_size = Size {
            width: 200,
            height: 120,
        };
        w.max_size = Size {
            width: 500,
            height: 400,
        };
        scene.add_window(w).unwrap();
        scene
            .move_window(WindowId(1), Point { x: 900, y: -20 })
            .unwrap();
        assert_eq!(scene.window(WindowId(1)).unwrap().bounds.x, 500);
        assert_eq!(scene.window(WindowId(1)).unwrap().bounds.y, 0);
        scene
            .resize_window(
                WindowId(1),
                Size {
                    width: 900,
                    height: 10,
                },
            )
            .unwrap();
        assert_eq!(
            scene.window(WindowId(1)).unwrap().bounds,
            Rect::new(500, 0, 300, 120)
        );
    }
    #[test]
    fn maximize_then_restore_preserves_previous_bounds() {
        let mut scene = SceneGraph::new(Rect::new(0, 0, 1280, 720));
        scene.add_window(window(1, 20, 30, 400, 300)).unwrap();
        scene.maximize(WindowId(1)).unwrap();
        assert_eq!(
            scene.window(WindowId(1)).unwrap().bounds,
            Rect::new(0, 0, 1280, 720)
        );
        scene.restore(WindowId(1)).unwrap();
        assert_eq!(
            scene.window(WindowId(1)).unwrap().bounds,
            Rect::new(20, 30, 400, 300)
        );
    }
    #[test]
    fn close_focuses_next_topmost_window() {
        let mut scene = SceneGraph::new(Rect::new(0, 0, 800, 600));
        scene.add_window(window(1, 0, 0, 300, 300)).unwrap();
        scene.add_window(window(2, 0, 0, 300, 300)).unwrap();
        assert_eq!(scene.close(WindowId(2)).unwrap().state, WindowState::Closed);
        assert_eq!(scene.focused(), Some(WindowId(1)));
    }
    #[test]
    fn damage_is_clipped_and_coalesced() {
        let mut damage = DamageTracker::new();
        let screen = Rect::new(0, 0, 100, 100);
        damage.add(Rect::new(-10, -10, 20, 20), screen);
        damage.add(Rect::new(10, 0, 20, 10), screen);
        assert_eq!(damage.rects(), &[Rect::new(0, 0, 30, 10)]);
    }
    #[test]
    fn overflowing_damage_falls_back_to_full_repaint() {
        let mut damage = DamageTracker::new();
        let screen = Rect::new(0, 0, 1000, 1000);
        for i in 0..=MAX_DAMAGE_RECTS {
            damage.add(Rect::new((i as i32) * 3, 0, 1, 1), screen);
        }
        assert_eq!(damage.rects(), &[screen]);
    }
    #[test]
    fn pointer_capture_survives_pointer_leaving_window() {
        let mut scene = SceneGraph::new(Rect::new(0, 0, 800, 600));
        scene.add_window(window(1, 10, 10, 200, 200)).unwrap();
        let mut input = InputRouter::new();
        assert_eq!(
            input.route(&mut scene, InputEvent::PointerDown(Point { x: 20, y: 20 })),
            InputTarget::Window(WindowId(1))
        );
        assert_eq!(
            input.route(
                &mut scene,
                InputEvent::PointerMove(Point { x: 700, y: 500 })
            ),
            InputTarget::Window(WindowId(1))
        );
        assert_eq!(
            input.route(&mut scene, InputEvent::PointerUp(Point { x: 700, y: 500 })),
            InputTarget::Window(WindowId(1))
        );
        assert_eq!(input.captured(), None);
    }
    #[test]
    fn keyboard_routes_to_focus_and_tab_is_global() {
        let mut scene = SceneGraph::new(Rect::new(0, 0, 800, 600));
        scene.add_window(window(1, 0, 0, 100, 100)).unwrap();
        let mut input = InputRouter::new();
        assert_eq!(
            input.route(&mut scene, InputEvent::KeyDown(Key::Enter)),
            InputTarget::Window(WindowId(1))
        );
        assert_eq!(
            input.route(&mut scene, InputEvent::KeyDown(Key::Tab)),
            InputTarget::GlobalShortcut
        );
    }
    #[test]
    fn session_restore_restarts_previously_live_apps() {
        let mut apps = AppLifecycle::new();
        apps.launch(AppId(7)).unwrap();
        apps.set_active_window(AppId(7), Some(WindowId(2))).unwrap();
        let snapshot = apps.snapshot();
        let mut restored = AppLifecycle::new();
        restored.restore(&snapshot);
        assert_eq!(
            restored.session(AppId(7)).unwrap().state,
            AppState::Starting
        );
        assert_eq!(
            restored.session(AppId(7)).unwrap().active_window,
            Some(WindowId(2))
        );
    }
    #[test]
    fn application_transition_increments_revision() {
        let mut apps = AppLifecycle::new();
        apps.launch(AppId(1)).unwrap();
        let before = apps.session(AppId(1)).unwrap().revision;
        apps.transition(AppId(1), AppState::Suspended).unwrap();
        assert_eq!(apps.session(AppId(1)).unwrap().revision, before + 1);
    }
    #[test]
    fn notification_duplicate_updates_in_place() {
        let mut center = NotificationCenter::new();
        center.push(
            Notification::new(
                1,
                AppId(1),
                "Оновлення",
                "Завантаження",
                NotificationUrgency::Normal,
                None,
            )
            .unwrap(),
        );
        center.push(
            Notification::new(
                1,
                AppId(1),
                "Готово",
                "Установлено",
                NotificationUrgency::Normal,
                None,
            )
            .unwrap(),
        );
        assert_eq!(center.len(), 1);
        assert_eq!(center.get(0).unwrap().title.as_str(), "Готово");
    }
    #[test]
    fn notifications_expire_and_dismiss_in_order() {
        let mut center = NotificationCenter::new();
        center.push(
            Notification::new(
                1,
                AppId(1),
                "Перше",
                "Текст",
                NotificationUrgency::Low,
                Some(10),
            )
            .unwrap(),
        );
        center.push(
            Notification::new(
                2,
                AppId(1),
                "Друге",
                "Текст",
                NotificationUrgency::Critical,
                None,
            )
            .unwrap(),
        );
        center.expire(10);
        assert_eq!(center.len(), 1);
        assert_eq!(center.get(0).unwrap().id, 2);
        assert!(center.dismiss(2));
        assert_eq!(center.len(), 0);
    }
    #[test]
    fn notification_ring_discards_oldest_when_full() {
        let mut center = NotificationCenter::new();
        for id in 0..=MAX_NOTIFICATIONS as u64 {
            center.push(
                Notification::new(
                    id,
                    AppId(1),
                    "Подія",
                    "Текст",
                    NotificationUrgency::Low,
                    None,
                )
                .unwrap(),
            );
        }
        assert_eq!(center.len(), MAX_NOTIFICATIONS);
        assert_eq!(center.get(0).unwrap().id, 1);
    }
}
