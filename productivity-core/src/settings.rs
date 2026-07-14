use crate::Text;

pub const MAX_SETTINGS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    Тема,
    Мова,
    Масштаб,
    Гучність,
    Яскравість,
    Мережа,
    Шпалери,
    Автозбереження,
    Власний(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingValue {
    Перемикач(bool),
    Число(i64),
    Текст(Text<128>),
    Колір(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    key: SettingKey,
    value: SettingValue,
}

pub struct Settings {
    entries: [Option<Entry>; MAX_SETTINGS],
    generation: u64,
}

impl Settings {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_SETTINGS],
            generation: 0,
        }
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub fn get(&self, key: SettingKey) -> Option<SettingValue> {
        self.entries
            .iter()
            .flatten()
            .find(|e| e.key == key)
            .map(|e| e.value)
    }
    pub fn begin(&self) -> SettingsTransaction {
        SettingsTransaction {
            base_generation: self.generation,
            changes: [None; 16],
            len: 0,
        }
    }
    fn apply(&mut self, tx: SettingsTransaction) -> Result<(), &'static str> {
        if tx.base_generation != self.generation {
            return Err("Налаштування вже змінено в іншому вікні");
        }
        let free = self.entries.iter().filter(|entry| entry.is_none()).count();
        let required = tx.changes[..tx.len]
            .iter()
            .flatten()
            .filter(|change| {
                !self
                    .entries
                    .iter()
                    .flatten()
                    .any(|entry| entry.key == change.key)
            })
            .count();
        if required > free {
            return Err("Сховище налаштувань заповнене");
        }
        for entry in tx.changes[..tx.len].iter().flatten() {
            if let Some(existing) = self
                .entries
                .iter_mut()
                .flatten()
                .find(|e| e.key == entry.key)
            {
                existing.value = entry.value;
            } else if let Some(slot) = self.entries.iter_mut().find(|e| e.is_none()) {
                *slot = Some(*entry);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SettingsTransaction {
    base_generation: u64,
    changes: [Option<Entry>; 16],
    len: usize,
}
impl SettingsTransaction {
    pub fn set(&mut self, key: SettingKey, value: SettingValue) -> Result<(), &'static str> {
        if let Some(existing) = self.changes[..self.len]
            .iter_mut()
            .flatten()
            .find(|e| e.key == key)
        {
            existing.value = value;
            return Ok(());
        }
        if self.len == self.changes.len() {
            return Err("Забагато змін в одній операції");
        }
        self.changes[self.len] = Some(Entry { key, value });
        self.len += 1;
        Ok(())
    }
    pub fn commit(self, settings: &mut Settings) -> Result<(), &'static str> {
        settings.apply(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_commit_and_conflict() {
        let mut s = Settings::new();
        let mut a = s.begin();
        let mut stale = s.begin();
        a.set(
            SettingKey::Мова,
            SettingValue::Текст(Text::new("Українська").unwrap()),
        )
        .unwrap();
        a.commit(&mut s).unwrap();
        stale
            .set(
                SettingKey::Тема,
                SettingValue::Текст(Text::new("Темна").unwrap()),
            )
            .unwrap();
        assert_eq!(
            stale.commit(&mut s),
            Err("Налаштування вже змінено в іншому вікні")
        );
    }
}
