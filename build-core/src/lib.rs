#![no_std]

//! Allocation-free core of the native Nova SDK, builder and package manager.

use core::fmt;

pub const MAX_PACKAGES: usize = 32;
pub const MAX_DEPENDENCIES: usize = 96;
pub const MAX_SOURCES: usize = 128;
pub const MAX_FILES: usize = 64;
pub const MAX_INSTALL_ACTIONS: usize = 96;

pub type Digest = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Executable,
    Library,
    KernelModule,
}

impl TargetKind {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "bin" => Some(Self::Executable),
            "lib" => Some(Self::Library),
            "kmod" => Some(Self::KernelModule),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Package<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub kind: TargetKind,
    pub entry: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dependency<'a> {
    pub package: &'a str,
    pub requires: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source<'a> {
    pub package: &'a str,
    pub path: &'a str,
}

const EMPTY_PACKAGE: Package<'static> = Package {
    name: "",
    version: "",
    kind: TargetKind::Library,
    entry: "",
};
const EMPTY_DEP: Dependency<'static> = Dependency {
    package: "",
    requires: "",
};
const EMPTY_SOURCE: Source<'static> = Source {
    package: "",
    path: "",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestError {
    MissingWorkspace,
    DuplicateWorkspace,
    UnknownDirective,
    WrongFieldCount,
    InvalidName,
    InvalidVersion,
    InvalidTarget,
    Capacity,
    DuplicatePackage,
    UnknownPackage,
    SelfDependency,
    DuplicateDependency,
}

impl ManifestError {
    pub const fn ukrainian(self) -> &'static str {
        match self {
            Self::MissingWorkspace => "не вказано робочий простір",
            Self::DuplicateWorkspace => "робочий простір оголошено двічі",
            Self::UnknownDirective => "невідома директива маніфесту",
            Self::WrongFieldCount => "неправильна кількість полів",
            Self::InvalidName => "неприпустима назва пакунка",
            Self::InvalidVersion => "неприпустима версія пакунка",
            Self::InvalidTarget => "непідтримуваний тип цілі",
            Self::Capacity => "маніфест перевищує системний ліміт",
            Self::DuplicatePackage => "пакунок оголошено двічі",
            Self::UnknownPackage => "залежність посилається на невідомий пакунок",
            Self::SelfDependency => "пакунок не може залежати від себе",
            Self::DuplicateDependency => "залежність оголошено двічі",
        }
    }
}

/// Text format (one directive per line):
/// `workspace NAME`, `package NAME VERSION bin|lib|kmod ENTRY`,
/// `dep PACKAGE REQUIRED_PACKAGE`, `source PACKAGE PATH`.
pub struct WorkspaceManifest<'a> {
    pub name: &'a str,
    packages: [Package<'a>; MAX_PACKAGES],
    package_len: usize,
    dependencies: [Dependency<'a>; MAX_DEPENDENCIES],
    dependency_len: usize,
    sources: [Source<'a>; MAX_SOURCES],
    source_len: usize,
}

impl<'a> WorkspaceManifest<'a> {
    pub fn parse(text: &'a str) -> Result<Self, ManifestError> {
        let mut out = Self {
            name: "",
            packages: [EMPTY_PACKAGE; MAX_PACKAGES],
            package_len: 0,
            dependencies: [EMPTY_DEP; MAX_DEPENDENCIES],
            dependency_len: 0,
            sources: [EMPTY_SOURCE; MAX_SOURCES],
            source_len: 0,
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_ascii_whitespace();
            match words.next().ok_or(ManifestError::UnknownDirective)? {
                "workspace" => {
                    if !out.name.is_empty() {
                        return Err(ManifestError::DuplicateWorkspace);
                    }
                    out.name = words.next().ok_or(ManifestError::WrongFieldCount)?;
                    if words.next().is_some() {
                        return Err(ManifestError::WrongFieldCount);
                    }
                    valid_name(out.name)?;
                }
                "package" => {
                    let name = words.next().ok_or(ManifestError::WrongFieldCount)?;
                    let version = words.next().ok_or(ManifestError::WrongFieldCount)?;
                    let kind =
                        TargetKind::parse(words.next().ok_or(ManifestError::WrongFieldCount)?)
                            .ok_or(ManifestError::InvalidTarget)?;
                    let entry = words.next().ok_or(ManifestError::WrongFieldCount)?;
                    if words.next().is_some() {
                        return Err(ManifestError::WrongFieldCount);
                    }
                    valid_name(name)?;
                    valid_version(version)?;
                    if out.packages().iter().any(|p| p.name == name) {
                        return Err(ManifestError::DuplicatePackage);
                    }
                    if out.package_len == MAX_PACKAGES {
                        return Err(ManifestError::Capacity);
                    }
                    out.packages[out.package_len] = Package {
                        name,
                        version,
                        kind,
                        entry,
                    };
                    out.package_len += 1;
                }
                "dep" => {
                    let package = words.next().ok_or(ManifestError::WrongFieldCount)?;
                    let requires = words.next().ok_or(ManifestError::WrongFieldCount)?;
                    if words.next().is_some() {
                        return Err(ManifestError::WrongFieldCount);
                    }
                    valid_name(package)?;
                    valid_name(requires)?;
                    if package == requires {
                        return Err(ManifestError::SelfDependency);
                    }
                    if out
                        .dependencies()
                        .iter()
                        .any(|d| d.package == package && d.requires == requires)
                    {
                        return Err(ManifestError::DuplicateDependency);
                    }
                    if out.dependency_len == MAX_DEPENDENCIES {
                        return Err(ManifestError::Capacity);
                    }
                    out.dependencies[out.dependency_len] = Dependency { package, requires };
                    out.dependency_len += 1;
                }
                "source" => {
                    let package = words.next().ok_or(ManifestError::WrongFieldCount)?;
                    let path = words.next().ok_or(ManifestError::WrongFieldCount)?;
                    if words.next().is_some() {
                        return Err(ManifestError::WrongFieldCount);
                    }
                    if out.source_len == MAX_SOURCES {
                        return Err(ManifestError::Capacity);
                    }
                    out.sources[out.source_len] = Source { package, path };
                    out.source_len += 1;
                }
                _ => return Err(ManifestError::UnknownDirective),
            }
        }
        if out.name.is_empty() {
            return Err(ManifestError::MissingWorkspace);
        }
        for dep in out.dependencies() {
            if out.package_index(dep.package).is_none() || out.package_index(dep.requires).is_none()
            {
                return Err(ManifestError::UnknownPackage);
            }
        }
        for source in out.sources() {
            if out.package_index(source.package).is_none() {
                return Err(ManifestError::UnknownPackage);
            }
        }
        Ok(out)
    }

    pub fn packages(&self) -> &[Package<'a>] {
        &self.packages[..self.package_len]
    }
    pub fn dependencies(&self) -> &[Dependency<'a>] {
        &self.dependencies[..self.dependency_len]
    }
    pub fn sources(&self) -> &[Source<'a>] {
        &self.sources[..self.source_len]
    }
    pub fn package_index(&self, name: &str) -> Option<usize> {
        self.packages().iter().position(|p| p.name == name)
    }

    pub fn topological_order(&self) -> Result<BuildOrder, GraphError> {
        let mut indegree = [0u8; MAX_PACKAGES];
        for dep in self.dependencies() {
            indegree[self.package_index(dep.package).unwrap()] += 1;
        }
        let mut result = BuildOrder {
            indices: [0; MAX_PACKAGES],
            len: 0,
        };
        let mut emitted = [false; MAX_PACKAGES];
        while result.len < self.package_len {
            let next = (0..self.package_len)
                .find(|&i| !emitted[i] && indegree[i] == 0)
                .ok_or(GraphError::Cycle)?;
            emitted[next] = true;
            result.indices[result.len] = next;
            result.len += 1;
            let built = self.packages[next].name;
            for dep in self.dependencies().iter().filter(|d| d.requires == built) {
                indegree[self.package_index(dep.package).unwrap()] -= 1;
            }
        }
        Ok(result)
    }
}

fn valid_name(s: &str) -> Result<(), ManifestError> {
    if s.is_empty()
        || s.len() > 63
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Err(ManifestError::InvalidName)
    } else {
        Ok(())
    }
}
fn valid_version(s: &str) -> Result<(), ManifestError> {
    let mut parts = s.split('.');
    if (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    }) && parts.next().is_none()
    {
        Ok(())
    } else {
        Err(ManifestError::InvalidVersion)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphError {
    Cycle,
}
impl GraphError {
    pub const fn ukrainian(self) -> &'static str {
        "виявлено циклічну залежність"
    }
}

pub struct BuildOrder {
    indices: [usize; MAX_PACKAGES],
    len: usize,
}
impl BuildOrder {
    pub fn indices(&self) -> &[usize] {
        &self.indices[..self.len]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceDigest<'a> {
    pub path: &'a str,
    pub digest: Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompilerConfig<'a> {
    pub compiler_id: &'a str,
    pub target: &'a str,
    pub profile: &'a str,
    pub flags: &'a str,
}

/// Content address includes every semantic input and uses length-delimited fields.
pub fn build_cache_key(
    package: Package<'_>,
    config: CompilerConfig<'_>,
    sources: &[SourceDigest<'_>],
    dependency_keys: &[Digest],
) -> Digest {
    let mut h = Sha256::new();
    h.field(b"nova-build-v1");
    h.field(package.name.as_bytes());
    h.field(package.version.as_bytes());
    h.field(&[package.kind as u8]);
    h.field(package.entry.as_bytes());
    h.field(config.compiler_id.as_bytes());
    h.field(config.target.as_bytes());
    h.field(config.profile.as_bytes());
    h.field(config.flags.as_bytes());
    for source in sources {
        h.field(source.path.as_bytes());
        h.field(&source.digest);
    }
    for dep in dependency_keys {
        h.field(dep);
    }
    h.finish()
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Compile = 1,
    Link = 2,
}
pub const JOB_PROTOCOL_VERSION: u16 = 1;
pub const JOB_HEADER_BYTES: usize = 48;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobHeader {
    pub kind: JobKind,
    pub id: u64,
    pub cache_key: Digest,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    ShortMessage,
    BadMagic,
    UnsupportedVersion,
    UnknownKind,
}
impl JobHeader {
    /// Stable little-endian wire header shared by the compiler service and linker service.
    pub fn encode(self) -> [u8; JOB_HEADER_BYTES] {
        let mut out = [0u8; JOB_HEADER_BYTES];
        out[..4].copy_from_slice(b"NJOB");
        out[4..6].copy_from_slice(&JOB_PROTOCOL_VERSION.to_le_bytes());
        out[6] = self.kind as u8;
        out[8..16].copy_from_slice(&self.id.to_le_bytes());
        out[16..48].copy_from_slice(&self.cache_key);
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() < JOB_HEADER_BYTES {
            return Err(ProtocolError::ShortMessage);
        }
        if &bytes[..4] != b"NJOB" {
            return Err(ProtocolError::BadMagic);
        }
        if u16::from_le_bytes([bytes[4], bytes[5]]) != JOB_PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        let kind = match bytes[6] {
            1 => JobKind::Compile,
            2 => JobKind::Link,
            _ => return Err(ProtocolError::UnknownKind),
        };
        let id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let mut cache_key = [0; 32];
        cache_key.copy_from_slice(&bytes[16..48]);
        Ok(Self {
            kind,
            id,
            cache_key,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileJob<'a> {
    pub id: u64,
    pub package: &'a str,
    pub source: &'a str,
    pub target: &'a str,
    pub output: &'a str,
    pub cache_key: Digest,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkJob<'a> {
    pub id: u64,
    pub package: &'a str,
    pub target: &'a str,
    pub output: &'a str,
    pub object_count: u16,
    pub cache_key: Digest,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobResult {
    pub id: u64,
    pub status: JobStatus,
    pub output_hash: Digest,
    pub diagnostic_code: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Note,
    Warning,
    Error,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostic<'a> {
    pub code: u16,
    pub severity: Severity,
    pub file: &'a str,
    pub line: u32,
    pub column: u32,
    pub message: &'a str,
}
impl<'a> Diagnostic<'a> {
    pub const fn ukrainian(
        code: u16,
        severity: Severity,
        file: &'a str,
        line: u32,
        column: u32,
    ) -> Self {
        let message = match code {
            1 => "не вдалося прочитати вихідний файл",
            2 => "помилка синтаксису",
            3 => "невідомий символ",
            4 => "не знайдено залежність",
            5 => "помилка компонування",
            6 => "підпис пакунка недійсний",
            7 => "кеш складання пошкоджено",
            _ => "невідома помилка складання",
        };
        Self {
            code,
            severity,
            file,
            line,
            column,
            message,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleFile<'a> {
    pub path: &'a str,
    pub size: u64,
    pub hash: Digest,
    pub executable: bool,
}
const EMPTY_FILE: BundleFile<'static> = BundleFile {
    path: "",
    size: 0,
    hash: [0; 32],
    executable: false,
};

pub struct BundleManifest<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub architecture: &'a str,
    files: [BundleFile<'a>; MAX_FILES],
    file_len: usize,
    pub payload_hash: Digest,
    pub signer_key_id: Digest,
    pub signature: [u8; 64],
}
impl<'a> BundleManifest<'a> {
    pub fn new(
        name: &'a str,
        version: &'a str,
        architecture: &'a str,
    ) -> Result<Self, ManifestError> {
        valid_name(name)?;
        valid_version(version)?;
        Ok(Self {
            name,
            version,
            architecture,
            files: [EMPTY_FILE; MAX_FILES],
            file_len: 0,
            payload_hash: [0; 32],
            signer_key_id: [0; 32],
            signature: [0; 64],
        })
    }
    pub fn add_file(&mut self, file: BundleFile<'a>) -> Result<(), PackageError> {
        if file.path.is_empty() || file.path.starts_with('/') || file.path.contains("..") {
            return Err(PackageError::UnsafePath);
        }
        if self.files().iter().any(|f| f.path == file.path) {
            return Err(PackageError::DuplicateFile);
        }
        if self.file_len == MAX_FILES {
            return Err(PackageError::Capacity);
        }
        self.files[self.file_len] = file;
        self.file_len += 1;
        Ok(())
    }
    pub fn files(&self) -> &[BundleFile<'a>] {
        &self.files[..self.file_len]
    }
    pub fn compute_payload_hash(&self) -> Digest {
        let mut h = Sha256::new();
        h.field(b"nova-bundle-v1");
        h.field(self.name.as_bytes());
        h.field(self.version.as_bytes());
        h.field(self.architecture.as_bytes());
        for f in self.files() {
            h.field(f.path.as_bytes());
            h.field(&f.size.to_le_bytes());
            h.field(&f.hash);
            h.field(&[f.executable as u8]);
        }
        h.finish()
    }
    pub fn verify<V: SignatureVerifier>(&self, verifier: &V) -> Result<(), PackageError> {
        let actual = self.compute_payload_hash();
        if !constant_time_eq(&actual, &self.payload_hash) {
            return Err(PackageError::HashMismatch);
        }
        if !verifier.verify(&self.signer_key_id, &actual, &self.signature) {
            return Err(PackageError::BadSignature);
        }
        Ok(())
    }
}

pub trait SignatureVerifier {
    fn verify(&self, key_id: &Digest, message: &Digest, signature: &[u8; 64]) -> bool;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageError {
    UnsafePath,
    DuplicateFile,
    Capacity,
    HashMismatch,
    BadSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallAction<'a> {
    BeginTransaction,
    CreateDir(&'a str),
    BackupFile(&'a str),
    WriteFile { path: &'a str, hash: Digest },
    SetExecutable(&'a str),
    RemoveFile(&'a str),
    CommitDatabase,
}
const EMPTY_ACTION: InstallAction<'static> = InstallAction::CommitDatabase;
pub struct InstallPlan<'a> {
    actions: [InstallAction<'a>; MAX_INSTALL_ACTIONS],
    len: usize,
}
impl<'a> InstallPlan<'a> {
    pub const fn new() -> Self {
        Self {
            actions: [EMPTY_ACTION; MAX_INSTALL_ACTIONS],
            len: 0,
        }
    }
    pub fn push(&mut self, action: InstallAction<'a>) -> Result<(), PackageError> {
        if self.len == MAX_INSTALL_ACTIONS {
            return Err(PackageError::Capacity);
        }
        self.actions[self.len] = action;
        self.len += 1;
        Ok(())
    }
    pub fn actions(&self) -> &[InstallAction<'a>] {
        &self.actions[..self.len]
    }
    /// Creates an atomic plan: payload first, executable bits second, database commit last.
    pub fn for_bundle(bundle: &'a BundleManifest<'a>) -> Result<Self, PackageError> {
        let mut plan = Self::new();
        plan.push(InstallAction::BeginTransaction)?;
        for f in bundle.files() {
            plan.push(InstallAction::BackupFile(f.path))?;
            plan.push(InstallAction::WriteFile {
                path: f.path,
                hash: f.hash,
            })?;
        }
        for f in bundle.files().iter().filter(|f| f.executable) {
            plan.push(InstallAction::SetExecutable(f.path))?;
        }
        plan.push(InstallAction::CommitDatabase)?;
        Ok(plan)
    }
}
impl Default for InstallPlan<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditRecord {
    pub sequence: u64,
    pub source_tree: Digest,
    pub graph_key: Digest,
    pub output: Digest,
    pub compiler: Digest,
    pub started_tick: u64,
    pub finished_tick: u64,
    pub reproducible: bool,
}
impl AuditRecord {
    pub fn fingerprint(&self) -> Digest {
        let mut h = Sha256::new();
        h.field(b"nova-audit-v1");
        h.field(&self.sequence.to_le_bytes());
        h.field(&self.source_tree);
        h.field(&self.graph_key);
        h.field(&self.output);
        h.field(&self.compiler);
        h.field(&self.started_tick.to_le_bytes());
        h.field(&self.finished_tick.to_le_bytes());
        h.field(&[self.reproducible as u8]);
        h.finish()
    }
    pub fn matches_rebuild(&self, rebuilt: &Self) -> bool {
        self.source_tree == rebuilt.source_tree
            && self.graph_key == rebuilt.graph_key
            && self.compiler == rebuilt.compiler
            && self.output == rebuilt.output
    }
}

fn constant_time_eq(a: &Digest, b: &Digest) -> bool {
    let mut d = 0u8;
    for i in 0..32 {
        d |= a[i] ^ b[i];
    }
    d == 0
}

/// Small internal SHA-256 so the build core has no host or third-party runtime dependency.
pub fn sha256(data: &[u8]) -> Digest {
    let mut h = Sha256::new();
    h.update(data);
    h.finish()
}
struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    used: usize,
    bytes: u64,
}
impl Sha256 {
    const fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            used: 0,
            bytes: 0,
        }
    }
    fn field(&mut self, bytes: &[u8]) {
        self.update(&(bytes.len() as u64).to_le_bytes());
        self.update(bytes);
    }
    fn update(&mut self, mut input: &[u8]) {
        self.bytes += input.len() as u64;
        while !input.is_empty() {
            let n = core::cmp::min(64 - self.used, input.len());
            self.block[self.used..self.used + n].copy_from_slice(&input[..n]);
            self.used += n;
            input = &input[n..];
            if self.used == 64 {
                self.compress();
                self.used = 0;
            }
        }
    }
    fn finish(mut self) -> Digest {
        let bits = self.bytes * 8;
        self.block[self.used] = 0x80;
        self.used += 1;
        if self.used > 56 {
            self.block[self.used..].fill(0);
            self.compress();
            self.used = 0;
        }
        self.block[self.used..56].fill(0);
        self.block[56..].copy_from_slice(&bits.to_be_bytes());
        self.compress();
        let mut out = [0; 32];
        for (chunk, v) in out.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&v.to_be_bytes());
        }
        out
    }
    fn compress(&mut self) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for (i, c) in self.block.chunks_exact(4).take(16).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *s = s.wrapping_add(v);
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.ukrainian())
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    const GOOD: &str = "# Nova SDK\nworkspace nova\npackage base 1.0.0 lib src/base.rs\npackage editor 2.1.0 bin src/main.rs\ndep editor base\nsource base src/base.rs\nsource editor src/main.rs\n";

    #[test]
    fn parses_manifest() {
        let m = WorkspaceManifest::parse(GOOD).unwrap();
        assert_eq!(m.name, "nova");
        assert_eq!(m.packages().len(), 2);
        assert_eq!(m.sources().len(), 2);
    }
    #[test]
    fn rejects_unknown_reference() {
        assert_eq!(
            WorkspaceManifest::parse("workspace x\npackage a 1.0.0 lib a.rs\ndep a z").err(),
            Some(ManifestError::UnknownPackage)
        );
    }
    #[test]
    fn topological_dependencies_first() {
        let m = WorkspaceManifest::parse(GOOD).unwrap();
        assert_eq!(m.topological_order().unwrap().indices(), &[0, 1]);
    }
    #[test]
    fn rejects_cycle() {
        let m = WorkspaceManifest::parse(
            "workspace x\npackage a 1.0.0 lib a\npackage b 1.0.0 lib b\ndep a b\ndep b a",
        )
        .unwrap();
        assert_eq!(m.topological_order().err(), Some(GraphError::Cycle));
    }
    #[test]
    fn sha_vector() {
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
    }
    #[test]
    fn cache_is_deterministic_and_sensitive() {
        let p = WorkspaceManifest::parse(GOOD).unwrap().packages()[0];
        let c = CompilerConfig {
            compiler_id: "nova-rust-1",
            target: "x86_64-nova",
            profile: "release",
            flags: "",
        };
        let s = [SourceDigest {
            path: "a",
            digest: sha256(b"one"),
        }];
        let a = build_cache_key(p, c, &s, &[]);
        assert_eq!(a, build_cache_key(p, c, &s, &[]));
        let s2 = [SourceDigest {
            path: "a",
            digest: sha256(b"two"),
        }];
        assert_ne!(a, build_cache_key(p, c, &s2, &[]));
    }
    struct Accept;
    impl SignatureVerifier for Accept {
        fn verify(&self, _: &Digest, m: &Digest, s: &[u8; 64]) -> bool {
            s[..32] == m[..]
        }
    }
    #[test]
    fn bundle_hash_and_signature() {
        let mut b = BundleManifest::new("editor", "1.2.3", "x86_64-nova").unwrap();
        b.add_file(BundleFile {
            path: "bin/editor",
            size: 3,
            hash: sha256(b"app"),
            executable: true,
        })
        .unwrap();
        b.payload_hash = b.compute_payload_hash();
        b.signature[..32].copy_from_slice(&b.payload_hash);
        assert_eq!(b.verify(&Accept), Ok(()));
    }
    #[test]
    fn bundle_detects_tamper() {
        let mut b = BundleManifest::new("editor", "1.2.3", "x86_64-nova").unwrap();
        b.add_file(BundleFile {
            path: "bin/editor",
            size: 3,
            hash: sha256(b"app"),
            executable: true,
        })
        .unwrap();
        assert_eq!(b.verify(&Accept), Err(PackageError::HashMismatch));
    }
    #[test]
    fn rejects_traversal() {
        let mut b = BundleManifest::new("x", "1.0.0", "x").unwrap();
        assert_eq!(
            b.add_file(BundleFile {
                path: "../x",
                size: 0,
                hash: [0; 32],
                executable: false
            }),
            Err(PackageError::UnsafePath)
        );
    }
    #[test]
    fn transaction_commits_last() {
        let mut b = BundleManifest::new("x", "1.0.0", "x").unwrap();
        b.add_file(BundleFile {
            path: "bin/x",
            size: 1,
            hash: [1; 32],
            executable: true,
        })
        .unwrap();
        let p = InstallPlan::for_bundle(&b).unwrap();
        assert_eq!(p.actions().len(), 5);
        assert!(matches!(p.actions()[0], InstallAction::BeginTransaction));
        assert!(matches!(p.actions()[4], InstallAction::CommitDatabase));
    }
    #[test]
    fn job_protocol_round_trip_and_validation() {
        let h = JobHeader {
            kind: JobKind::Link,
            id: 77,
            cache_key: [9; 32],
        };
        assert_eq!(JobHeader::decode(&h.encode()), Ok(h));
        let mut bad = h.encode();
        bad[0] = 0;
        assert_eq!(JobHeader::decode(&bad), Err(ProtocolError::BadMagic));
    }
    #[test]
    fn diagnostic_is_ukrainian() {
        assert_eq!(
            Diagnostic::ukrainian(5, Severity::Error, "x", 1, 1).message,
            "помилка компонування"
        );
    }
    #[test]
    fn audit_rebuild_comparison() {
        let a = AuditRecord {
            sequence: 1,
            source_tree: [1; 32],
            graph_key: [2; 32],
            output: [3; 32],
            compiler: [4; 32],
            started_tick: 1,
            finished_tick: 2,
            reproducible: true,
        };
        let mut b = a;
        b.sequence = 2;
        assert!(a.matches_rebuild(&b));
        b.output = [9; 32];
        assert!(!a.matches_rebuild(&b));
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
