use crate::Text;

pub const MAX_FILES: usize = 128;
pub const MAX_NAME: usize = 192;
pub const MAX_PATH: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileKind {
    Тека,
    Документ,
    Зображення,
    Аудіо,
    Відео,
    Програма,
    Інше,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileEntry {
    pub name: Text<MAX_NAME>,
    pub kind: FileKind,
    pub size: u64,
    pub modified_unix: u64,
    pub hidden: bool,
}

impl FileEntry {
    pub fn new(name: &str, kind: FileKind, size: u64, modified_unix: u64) -> Option<Self> {
        Some(Self {
            name: Text::new(name).ok()?,
            kind,
            size,
            modified_unix,
            hidden: false,
        })
    }
}

impl Default for FileEntry {
    fn default() -> Self {
        Self {
            name: Text::empty(),
            kind: FileKind::Інше,
            size: 0,
            modified_unix: 0,
            hidden: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Назва,
    Тип,
    Розмір,
    Змінено,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardAction {
    Копіювати,
    Перемістити,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileClipboard {
    pub source: Text<MAX_PATH>,
    pub action: ClipboardAction,
}

pub struct FileManager {
    entries: [FileEntry; MAX_FILES],
    len: usize,
    order: [u8; MAX_FILES],
    visible_len: usize,
    filter: Text<MAX_NAME>,
    show_hidden: bool,
    sort: SortField,
    ascending: bool,
    selected: Option<usize>,
    clipboard: Option<FileClipboard>,
}

impl FileManager {
    pub const fn new() -> Self {
        Self {
            entries: [FileEntry {
                name: Text::empty(),
                kind: FileKind::Інше,
                size: 0,
                modified_unix: 0,
                hidden: false,
            }; MAX_FILES],
            len: 0,
            order: [0; MAX_FILES],
            visible_len: 0,
            filter: Text::empty(),
            show_hidden: false,
            sort: SortField::Назва,
            ascending: true,
            selected: None,
            clipboard: None,
        }
    }

    pub fn load(&mut self, entries: &[FileEntry]) -> Result<(), &'static str> {
        if entries.len() > MAX_FILES {
            return Err("Забагато файлів у теці");
        }
        self.entries[..entries.len()].copy_from_slice(entries);
        self.len = entries.len();
        self.rebuild();
        Ok(())
    }

    pub fn set_filter(&mut self, filter: &str) -> Result<(), &'static str> {
        self.filter.set(filter).map_err(|_| "Фільтр надто довгий")?;
        self.rebuild();
        Ok(())
    }
    pub fn set_show_hidden(&mut self, value: bool) {
        self.show_hidden = value;
        self.rebuild();
    }
    pub fn sort_by(&mut self, field: SortField, ascending: bool) {
        self.sort = field;
        self.ascending = ascending;
        self.rebuild();
    }
    pub const fn visible_len(&self) -> usize {
        self.visible_len
    }
    pub fn visible(&self, index: usize) -> Option<&FileEntry> {
        self.order
            .get(index)
            .filter(|_| index < self.visible_len)
            .map(|i| &self.entries[*i as usize])
    }
    pub fn select(&mut self, visible_index: usize) -> bool {
        if visible_index >= self.visible_len {
            return false;
        }
        self.selected = Some(self.order[visible_index] as usize);
        true
    }
    pub fn selected(&self) -> Option<&FileEntry> {
        self.selected.map(|i| &self.entries[i])
    }
    pub fn copy_path(&mut self, path: &str, action: ClipboardAction) -> Result<(), &'static str> {
        self.clipboard = Some(FileClipboard {
            source: Text::new(path).map_err(|_| "Шлях надто довгий")?,
            action,
        });
        Ok(())
    }
    pub const fn clipboard(&self) -> Option<FileClipboard> {
        self.clipboard
    }
    pub fn clear_clipboard(&mut self) {
        self.clipboard = None;
    }

    fn rebuild(&mut self) {
        self.visible_len = 0;
        let query = self.filter.as_str();
        for i in 0..self.len {
            let e = &self.entries[i];
            if (!e.hidden || self.show_hidden) && contains_ignore_ascii_case(e.name.as_str(), query)
            {
                self.order[self.visible_len] = i as u8;
                self.visible_len += 1;
            }
        }
        for i in 1..self.visible_len {
            let key = self.order[i];
            let mut j = i;
            while j > 0
                && self
                    .compare(key as usize, self.order[j - 1] as usize)
                    .is_lt()
            {
                self.order[j] = self.order[j - 1];
                j -= 1;
            }
            self.order[j] = key;
        }
    }

    fn compare(&self, a: usize, b: usize) -> core::cmp::Ordering {
        let a = &self.entries[a];
        let b = &self.entries[b];
        let folders = b.kind.eq(&FileKind::Тека).cmp(&a.kind.eq(&FileKind::Тека));
        let raw = if folders != core::cmp::Ordering::Equal {
            folders
        } else {
            match self.sort {
                SortField::Назва => a.name.as_str().cmp(b.name.as_str()),
                SortField::Тип => a.kind.cmp(&b.kind),
                SortField::Розмір => a.size.cmp(&b.size),
                SortField::Змінено => a.modified_unix.cmp(&b.modified_unix),
            }
        };
        if self.ascending { raw } else { raw.reverse() }
    }
}

impl Default for FileManager {
    fn default() -> Self {
        Self::new()
    }
}

fn contains_ignore_ascii_case(value: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    value
        .as_bytes()
        .windows(query.len())
        .any(|w| w.eq_ignore_ascii_case(query.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folders_filter_sort_and_clipboard() {
        let mut hidden = FileEntry::new(".секрет", FileKind::Документ, 1, 0).unwrap();
        hidden.hidden = true;
        let items = [
            FileEntry::new("z.txt", FileKind::Документ, 20, 2).unwrap(),
            FileEntry::new("Тека", FileKind::Тека, 0, 1).unwrap(),
            hidden,
            FileEntry::new("a.txt", FileKind::Документ, 10, 3).unwrap(),
        ];
        let mut fm = FileManager::new();
        fm.load(&items).unwrap();
        assert_eq!(fm.visible(0).unwrap().kind, FileKind::Тека);
        fm.set_filter(".txt").unwrap();
        fm.sort_by(SortField::Розмір, false);
        assert_eq!(fm.visible(0).unwrap().name.as_str(), "z.txt");
        fm.copy_path("/дім/нотатки", ClipboardAction::Копіювати)
            .unwrap();
        assert_eq!(fm.clipboard().unwrap().source.as_str(), "/дім/нотатки");
    }
}
