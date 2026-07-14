use crate::Text;

pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024;
pub const MAX_LINES: usize = 1024;
pub const MAX_HISTORY: usize = 32;
pub const MAX_EDIT_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub active: usize,
}

impl Selection {
    pub const fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            active: offset,
        }
    }
    pub fn range(self) -> core::ops::Range<usize> {
        self.anchor.min(self.active)..self.anchor.max(self.active)
    }
    pub const fn is_caret(self) -> bool {
        self.anchor == self.active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentError {
    ДокументЗаповнений,
    ЗмінаЗавелика,
    НеправильнаПозиція,
    НеправильнийUtf8,
    ПорожнійПошук,
}

#[derive(Clone, Copy)]
struct Edit {
    at: usize,
    removed: Text<MAX_EDIT_BYTES>,
    inserted: Text<MAX_EDIT_BYTES>,
    before: Selection,
    after: Selection,
}

impl Edit {
    const fn empty() -> Self {
        Self {
            at: 0,
            removed: Text::empty(),
            inserted: Text::empty(),
            before: Selection::caret(0),
            after: Selection::caret(0),
        }
    }
}

pub struct Document {
    bytes: [u8; MAX_DOCUMENT_BYTES],
    len: usize,
    selection: Selection,
    lines: [usize; MAX_LINES],
    line_count: usize,
    history: [Edit; MAX_HISTORY],
    history_len: usize,
    history_cursor: usize,
    revision: u64,
    saved_revision: u64,
}

impl Document {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; MAX_DOCUMENT_BYTES],
            len: 0,
            selection: Selection::caret(0),
            lines: [0; MAX_LINES],
            line_count: 1,
            history: [Edit::empty(); MAX_HISTORY],
            history_len: 0,
            history_cursor: 0,
            revision: 0,
            saved_revision: 0,
        }
    }

    pub fn new(text: &str) -> Result<Self, DocumentError> {
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::ДокументЗаповнений);
        }
        let mut doc = Self::empty();
        doc.bytes[..text.len()].copy_from_slice(text.as_bytes());
        doc.len = text.len();
        doc.reindex_lines();
        Ok(doc)
    }

    pub fn text(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub const fn selection(&self) -> Selection {
        self.selection
    }
    pub const fn line_count(&self) -> usize {
        self.line_count
    }
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    pub const fn is_modified(&self) -> bool {
        self.revision != self.saved_revision
    }
    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn set_selection(&mut self, selection: Selection) -> Result<(), DocumentError> {
        self.validate_offset(selection.anchor)?;
        self.validate_offset(selection.active)?;
        self.selection = selection;
        Ok(())
    }

    pub fn line_range(&self, line: usize) -> Option<core::ops::Range<usize>> {
        if line >= self.line_count {
            return None;
        }
        let start = self.lines[line];
        let mut end = if line + 1 < self.line_count {
            self.lines[line + 1]
        } else {
            self.len
        };
        if end > start && self.bytes[end - 1] == b'\n' {
            end -= 1;
        }
        Some(start..end)
    }

    pub fn line_and_column(&self, offset: usize) -> Option<(usize, usize)> {
        if self.validate_offset(offset).is_err() {
            return None;
        }
        let mut low = 0;
        let mut high = self.line_count;
        while low + 1 < high {
            let mid = (low + high) / 2;
            if self.lines[mid] <= offset {
                low = mid;
            } else {
                high = mid;
            }
        }
        Some((low, self.text()[self.lines[low]..offset].chars().count()))
    }

    pub fn replace_selection(&mut self, text: &str) -> Result<(), DocumentError> {
        let range = self.selection.range();
        self.edit(range.start, range.end, text, true)
    }

    pub fn insert(&mut self, offset: usize, text: &str) -> Result<(), DocumentError> {
        self.edit(offset, offset, text, true)
    }

    pub fn delete(&mut self, range: core::ops::Range<usize>) -> Result<(), DocumentError> {
        self.edit(range.start, range.end, "", true)
    }

    pub fn search<'a>(
        &'a self,
        needle: &'a str,
        from: usize,
    ) -> Result<SearchMatches<'a>, DocumentError> {
        self.validate_offset(from)?;
        if needle.is_empty() {
            return Err(DocumentError::ПорожнійПошук);
        }
        Ok(SearchMatches {
            haystack: self.text(),
            needle,
            cursor: from,
        })
    }

    pub fn replace_next(
        &mut self,
        needle: &str,
        replacement: &str,
        from: usize,
    ) -> Result<Option<usize>, DocumentError> {
        let found = self.search(needle, from)?.next();
        if let Some(at) = found {
            self.edit(at, at + needle.len(), replacement, true)?;
        }
        Ok(found)
    }

    /// Замінює всі неперекривні збіги та повертає їх кількість.
    /// Місткість перевіряється до першої зміни, тому переповнення не залишає
    /// документ у частково зміненому стані.
    pub fn replace_all(&mut self, needle: &str, replacement: &str) -> Result<usize, DocumentError> {
        if needle.is_empty() {
            return Err(DocumentError::ПорожнійПошук);
        }
        if needle.len() > MAX_EDIT_BYTES || replacement.len() > MAX_EDIT_BYTES {
            return Err(DocumentError::ЗмінаЗавелика);
        }
        let count = self.text().match_indices(needle).count();
        let removed = needle
            .len()
            .checked_mul(count)
            .ok_or(DocumentError::ДокументЗаповнений)?;
        let added = replacement
            .len()
            .checked_mul(count)
            .ok_or(DocumentError::ДокументЗаповнений)?;
        let final_len = self
            .len
            .checked_sub(removed)
            .and_then(|len| len.checked_add(added))
            .ok_or(DocumentError::ДокументЗаповнений)?;
        if final_len > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::ДокументЗаповнений);
        }

        let mut cursor = 0;
        for _ in 0..count {
            let relative = self.text()[cursor..]
                .find(needle)
                .ok_or(DocumentError::НеправильнаПозиція)?;
            let at = cursor + relative;
            self.edit(at, at + needle.len(), replacement, true)?;
            cursor = at + replacement.len();
        }
        Ok(count)
    }

    pub fn undo(&mut self) -> bool {
        if self.history_cursor == 0 {
            return false;
        }
        self.history_cursor -= 1;
        let edit = self.history[self.history_cursor];
        if self
            .edit(
                edit.at,
                edit.at + edit.inserted.len(),
                edit.removed.as_str(),
                false,
            )
            .is_err()
        {
            return false;
        }
        self.selection = edit.before;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.history_cursor >= self.history_len {
            return false;
        }
        let edit = self.history[self.history_cursor];
        if self
            .edit(
                edit.at,
                edit.at + edit.removed.len(),
                edit.inserted.as_str(),
                false,
            )
            .is_err()
        {
            return false;
        }
        self.selection = edit.after;
        self.history_cursor += 1;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    fn edit(
        &mut self,
        start: usize,
        end: usize,
        inserted: &str,
        record: bool,
    ) -> Result<(), DocumentError> {
        self.validate_offset(start)?;
        self.validate_offset(end)?;
        if start > end {
            return Err(DocumentError::НеправильнаПозиція);
        }
        let removed_len = end - start;
        if removed_len > MAX_EDIT_BYTES || inserted.len() > MAX_EDIT_BYTES {
            return Err(DocumentError::ЗмінаЗавелика);
        }
        let new_len = self.len - removed_len + inserted.len();
        if new_len > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::ДокументЗаповнений);
        }
        let before = self.selection;
        let removed = Text::new(
            self.text()
                .get(start..end)
                .ok_or(DocumentError::НеправильнийUtf8)?,
        )
        .map_err(|_| DocumentError::ЗмінаЗавелика)?;
        if inserted.len() != removed_len {
            self.bytes
                .copy_within(end..self.len, start + inserted.len());
        }
        self.bytes[start..start + inserted.len()].copy_from_slice(inserted.as_bytes());
        self.len = new_len;
        let caret = start + inserted.len();
        self.selection = Selection::caret(caret);
        self.reindex_lines();
        if record {
            let edit = Edit {
                at: start,
                removed,
                inserted: Text::new(inserted)
                    .map_err(|_| DocumentError::ЗмінаЗавелика)?,
                before,
                after: self.selection,
            };
            if self.history_cursor == MAX_HISTORY {
                self.history.copy_within(1..MAX_HISTORY, 0);
                self.history_cursor -= 1;
                self.history_len -= 1;
            }
            self.history[self.history_cursor] = edit;
            self.history_cursor += 1;
            self.history_len = self.history_cursor;
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(())
    }

    fn validate_offset(&self, offset: usize) -> Result<(), DocumentError> {
        if offset > self.len || !self.text().is_char_boundary(offset) {
            Err(DocumentError::НеправильнаПозиція)
        } else {
            Ok(())
        }
    }

    fn reindex_lines(&mut self) {
        self.lines[0] = 0;
        self.line_count = 1;
        for (index, byte) in self.bytes[..self.len].iter().enumerate() {
            if *byte == b'\n' && self.line_count < MAX_LINES {
                self.lines[self.line_count] = index + 1;
                self.line_count += 1;
            }
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::empty()
    }
}

pub struct SearchMatches<'a> {
    haystack: &'a str,
    needle: &'a str,
    cursor: usize,
}
impl Iterator for SearchMatches<'_> {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        let relative = self.haystack.get(self.cursor..)?.find(self.needle)?;
        let found = self.cursor + relative;
        self.cursor = found + self.needle.len();
        Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_edit_selection_and_history() {
        let mut d = Document::new("Привіт, світе!").unwrap();
        d.set_selection(Selection {
            anchor: 14,
            active: 24,
        })
        .unwrap();
        d.replace_selection("Nova").unwrap();
        assert_eq!(d.text(), "Привіт, Nova!");
        assert!(d.undo());
        assert_eq!(d.text(), "Привіт, світе!");
        assert!(d.redo());
        assert_eq!(d.text(), "Привіт, Nova!");
    }

    #[test]
    fn lines_search_replace_and_modified() {
        let mut d = Document::new("раз\nдва\nдва").unwrap();
        assert_eq!(d.line_count(), 3);
        assert_eq!(&d.text()[d.line_range(1).unwrap()], "два");
        assert_eq!(
            d.search("два", 0).unwrap().collect::<std::vec::Vec<_>>(),
            [7, 14]
        );
        d.replace_next("два", "три", 0).unwrap();
        assert!(d.is_modified());
        d.mark_saved();
        assert!(!d.is_modified());
    }

    #[test]
    fn replace_all_is_utf8_safe() {
        let mut d = Document::new("так ні так").unwrap();
        assert_eq!(d.replace_all("так", "авжеж").unwrap(), 2);
        assert_eq!(d.text(), "авжеж ні авжеж");
    }

    #[test]
    fn rejects_middle_of_codepoint() {
        let mut d = Document::new("ї").unwrap();
        assert_eq!(d.insert(1, "x"), Err(DocumentError::НеправильнаПозиція));
    }
}
