use crate::Text;

pub const MAX_APPS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPhase {
    Запускається,
    Активна,
    Призупинена,
    Завершується,
    Завершена,
    Аварія,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppLifecycle {
    pub id: u32,
    pub name: Text<64>,
    pub phase: AppPhase,
    pub dirty: bool,
    pub last_activity_ms: u64,
    pub last_autosave_ms: u64,
    pub session_token: u64,
}

impl AppLifecycle {
    pub fn new(id: u32, name: &str, now_ms: u64) -> Result<Self, &'static str> {
        Ok(Self {
            id,
            name: Text::new(name).map_err(|_| "Назва програми надто довга")?,
            phase: AppPhase::Запускається,
            dirty: false,
            last_activity_ms: now_ms,
            last_autosave_ms: now_ms,
            session_token: 0,
        })
    }
    pub fn activate(&mut self, now_ms: u64) {
        self.phase = AppPhase::Активна;
        self.last_activity_ms = now_ms;
    }
    pub fn mark_dirty(&mut self, now_ms: u64) {
        self.dirty = true;
        self.last_activity_ms = now_ms;
    }
    pub fn autosave_due(&self, now_ms: u64, interval_ms: u64) -> bool {
        self.dirty && now_ms.saturating_sub(self.last_autosave_ms) >= interval_ms
    }
    pub fn autosaved(&mut self, now_ms: u64, token: u64) {
        self.dirty = false;
        self.last_autosave_ms = now_ms;
        self.session_token = token;
    }
}

pub struct SessionCoordinator {
    apps: [Option<AppLifecycle>; MAX_APPS],
    autosave_interval_ms: u64,
}
impl SessionCoordinator {
    pub const fn new(autosave_interval_ms: u64) -> Self {
        Self {
            apps: [None; MAX_APPS],
            autosave_interval_ms,
        }
    }
    pub fn register(&mut self, app: AppLifecycle) -> Result<(), &'static str> {
        if self.apps.iter().flatten().any(|a| a.id == app.id) {
            return Err("Програму вже зареєстровано");
        }
        let slot = self
            .apps
            .iter_mut()
            .find(|a| a.is_none())
            .ok_or("Забагато відкритих програм")?;
        *slot = Some(app);
        Ok(())
    }
    pub fn app_mut(&mut self, id: u32) -> Option<&mut AppLifecycle> {
        self.apps.iter_mut().flatten().find(|a| a.id == id)
    }
    pub fn next_autosave(&self, now_ms: u64) -> Option<u32> {
        self.apps
            .iter()
            .flatten()
            .find(|a| a.autosave_due(now_ms, self.autosave_interval_ms))
            .map(|a| a.id)
    }
    pub fn can_end_session(&self) -> bool {
        !self
            .apps
            .iter()
            .flatten()
            .any(|a| a.dirty || matches!(a.phase, AppPhase::Запускається | AppPhase::Завершується))
    }
    pub fn crash(&mut self, id: u32) -> bool {
        if let Some(app) = self.app_mut(id) {
            app.phase = AppPhase::Аварія;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn autosave_and_safe_shutdown() {
        let mut s = SessionCoordinator::new(5_000);
        let mut app = AppLifecycle::new(7, "Нотатки", 0).unwrap();
        app.activate(1);
        app.mark_dirty(2);
        s.register(app).unwrap();
        assert_eq!(s.next_autosave(5_001), Some(7));
        assert!(!s.can_end_session());
        s.app_mut(7).unwrap().autosaved(5_001, 42);
        assert!(s.can_end_session());
    }
}
