#![no_std]

pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024;
pub const MAX_JOBS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppId {
    Files,
    Terminal,
    Editor,
    Browser,
    Packages,
    ControlCenter,
    Recovery,
    NovaAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    OpenPath,
    Save,
    Build,
    Install,
    Update,
    Recover,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppMessage<'a> {
    pub source: AppId,
    pub target: AppId,
    pub event: AppEvent,
    pub payload: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorError {
    Full,
    InvalidPosition,
}

/// Allocation-free UTF-8 document buffer used by the native Nova Editor.
pub struct Document {
    bytes: [u8; MAX_DOCUMENT_BYTES],
    len: usize,
    revision: u64,
    saved_revision: u64,
}

impl Document {
    pub const fn new() -> Self {
        Self {
            bytes: [0; MAX_DOCUMENT_BYTES],
            len: 0,
            revision: 0,
            saved_revision: 0,
        }
    }

    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
    pub const fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn replace(&mut self, text: &str) -> Result<(), EditorError> {
        if text.len() > self.bytes.len() {
            return Err(EditorError::Full);
        }
        self.bytes[..text.len()].copy_from_slice(text.as_bytes());
        self.len = text.len();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn insert(&mut self, at: usize, text: &str) -> Result<(), EditorError> {
        if at > self.len || !self.text().is_char_boundary(at) {
            return Err(EditorError::InvalidPosition);
        }
        if self.len + text.len() > self.bytes.len() {
            return Err(EditorError::Full);
        }
        self.bytes.copy_within(at..self.len, at + text.len());
        self.bytes[at..at + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn delete(&mut self, range: core::ops::Range<usize>) -> Result<(), EditorError> {
        if range.start > range.end
            || range.end > self.len
            || !self.text().is_char_boundary(range.start)
            || !self.text().is_char_boundary(range.end)
        {
            return Err(EditorError::InvalidPosition);
        }
        self.bytes.copy_within(range.end..self.len, range.start);
        self.len -= range.end - range.start;
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageManifest<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub executable: &'a str,
    pub checksum: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestError {
    MissingField,
    InvalidName,
    InvalidChecksum,
}

impl<'a> PackageManifest<'a> {
    /// Parses Nova's compact native manifest: `name|version|executable|hex-checksum`.
    pub fn parse(source: &'a str) -> Result<Self, ManifestError> {
        let mut fields = source.split('|');
        let name = fields.next().ok_or(ManifestError::MissingField)?;
        let version = fields.next().ok_or(ManifestError::MissingField)?;
        let executable = fields.next().ok_or(ManifestError::MissingField)?;
        let checksum_text = fields.next().ok_or(ManifestError::MissingField)?;
        if fields.next().is_some() {
            return Err(ManifestError::InvalidName);
        }
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(ManifestError::InvalidName);
        }
        let checksum =
            u64::from_str_radix(checksum_text, 16).map_err(|_| ManifestError::InvalidChecksum)?;
        Ok(Self {
            name,
            version,
            executable,
            checksum,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildState {
    Queued,
    Compiling,
    Testing,
    Packaging,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildJob {
    pub id: u64,
    pub state: BuildState,
    pub progress: u8,
}

pub struct BuildQueue {
    jobs: [Option<BuildJob>; MAX_JOBS],
    next_id: u64,
}

impl BuildQueue {
    pub const fn new() -> Self {
        Self {
            jobs: [None; MAX_JOBS],
            next_id: 1,
        }
    }
    pub fn enqueue(&mut self) -> Option<u64> {
        let slot = self.jobs.iter_mut().find(|job| job.is_none())?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        *slot = Some(BuildJob {
            id,
            state: BuildState::Queued,
            progress: 0,
        });
        Some(id)
    }
    pub fn transition(&mut self, id: u64, state: BuildState, progress: u8) -> bool {
        let Some(job) = self.jobs.iter_mut().flatten().find(|job| job.id == id) else {
            return false;
        };
        job.state = state;
        job.progress = progress.min(100);
        true
    }
    pub fn get(&self, id: u64) -> Option<BuildJob> {
        self.jobs.iter().flatten().find(|job| job.id == id).copied()
    }
}

impl Default for BuildQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Staged,
    Verified,
    Switched,
    Committed,
    RolledBack,
}

pub struct UpdateTransaction {
    phase: UpdatePhase,
    expected: u64,
    observed: u64,
}

impl UpdateTransaction {
    pub const fn new() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            expected: 0,
            observed: 0,
        }
    }
    pub fn stage(&mut self, expected_checksum: u64, observed_checksum: u64) {
        self.expected = expected_checksum;
        self.observed = observed_checksum;
        self.phase = UpdatePhase::Staged;
    }
    pub fn verify(&mut self) -> bool {
        if self.phase == UpdatePhase::Staged && self.expected == self.observed {
            self.phase = UpdatePhase::Verified;
            true
        } else {
            false
        }
    }
    pub fn switch(&mut self) -> bool {
        if self.phase != UpdatePhase::Verified {
            return false;
        }
        self.phase = UpdatePhase::Switched;
        true
    }
    pub fn commit(&mut self) -> bool {
        if self.phase != UpdatePhase::Switched {
            return false;
        }
        self.phase = UpdatePhase::Committed;
        true
    }
    pub fn rollback(&mut self) {
        self.phase = UpdatePhase::RolledBack;
    }
    pub const fn phase(&self) -> UpdatePhase {
        self.phase
    }
}

impl Default for UpdateTransaction {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_handles_ukrainian_utf8_and_dirty_state() {
        let mut document = Document::new();
        document.replace("Нова ОС").unwrap();
        document.insert("Нова".len(), " автономна").unwrap();
        assert_eq!(document.text(), "Нова автономна ОС");
        assert!(document.is_dirty());
        document.mark_saved();
        assert!(!document.is_dirty());
    }

    #[test]
    fn build_queue_tracks_real_pipeline_state() {
        let mut queue = BuildQueue::new();
        let id = queue.enqueue().unwrap();
        assert!(queue.transition(id, BuildState::Testing, 70));
        assert_eq!(queue.get(id).unwrap().progress, 70);
    }

    #[test]
    fn update_requires_verified_image_before_switch() {
        let mut update = UpdateTransaction::new();
        update.stage(0x1234, 0x1234);
        assert!(update.verify());
        assert!(update.switch());
        assert!(update.commit());
        assert_eq!(update.phase(), UpdatePhase::Committed);
    }

    #[test]
    fn package_manifest_is_strict() {
        let manifest = PackageManifest::parse("nova-editor|0.1.0|/Програми/editor|ff00").unwrap();
        assert_eq!(manifest.name, "nova-editor");
        assert_eq!(manifest.checksum, 0xff00);
    }
}
