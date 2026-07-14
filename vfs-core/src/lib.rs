#![no_std]

//! Nova OS virtual filesystem core.
//!
//! The crate has no platform or standard-library dependency. Backends implement
//! [`Backend`]; [`Vfs`] supplies canonical paths, mounts, handles, permissions,
//! ACLs, quotas and bounded symbolic-link traversal.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cmp;

pub type InodeId = u64;
pub type UserId = u32;
pub type GroupId = u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VfsError {
    InvalidPath,
    NameTooLong,
    NotFound,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    DirectoryNotEmpty,
    PermissionDenied,
    ReadOnly,
    InvalidHandle,
    TooManySymlinks,
    CrossDevice,
    QuotaExceeded,
    InvalidArgument,
    NoSpace,
    Unsupported,
    Corrupt,
}

pub type Result<T> = core::result::Result<T, VfsError>;

/// Allocation-free first-pass validation used at syscall boundaries before a
/// canonical owned path is built. It rejects NUL, relative paths, overlong
/// components and attempts to walk above the VFS root.
pub fn validate_path_syntax(path: &str) -> Result<()> {
    if !path.starts_with('/') || path.as_bytes().contains(&0) || path.len() > 4096 {
        return Err(VfsError::InvalidPath);
    }
    let mut depth = 0usize;
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if depth == 0 {
                    return Err(VfsError::InvalidPath);
                }
                depth -= 1;
            }
            name => {
                if name.len() > 255 {
                    return Err(VfsError::NameTooLong);
                }
                depth += 1;
            }
        }
    }
    Ok(())
}

/// Absolute, normalized UTF-8 path. `.` and duplicate separators disappear;
/// `..` cannot escape the root. NUL and overlong components are rejected.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CanonicalPath(String);

impl CanonicalPath {
    pub fn new(path: &str) -> Result<Self> {
        if !path.starts_with('/') || path.as_bytes().contains(&0) {
            return Err(VfsError::InvalidPath);
        }
        let mut parts: Vec<&str> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(VfsError::InvalidPath);
                    }
                }
                value => {
                    if value.len() > 255 {
                        return Err(VfsError::NameTooLong);
                    }
                    parts.push(value);
                }
            }
        }
        let mut value = String::from("/");
        value.push_str(&parts.join("/"));
        if value.len() > 4096 {
            return Err(VfsError::NameTooLong);
        }
        Ok(Self(value))
    }

    pub fn root() -> Self {
        Self(String::from("/"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }

    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        let at = self.0.rfind('/').unwrap_or(0);
        if at == 0 {
            Some(Self::root())
        } else {
            Some(Self(self.0[..at].to_string()))
        }
    }

    pub fn file_name(&self) -> Option<&str> {
        if self.is_root() {
            None
        } else {
            self.0.rsplit('/').next()
        }
    }

    pub fn join(&self, tail: &str) -> Result<Self> {
        if tail.starts_with('/') {
            return Self::new(tail);
        }
        let mut joined = self.0.clone();
        if !joined.ends_with('/') {
            joined.push('/');
        }
        joined.push_str(tail);
        Self::new(&joined)
    }

    fn is_prefix_of(&self, path: &Self) -> bool {
        self.is_root()
            || path.0 == self.0
            || (path.0.starts_with(&self.0) && path.0.as_bytes().get(self.0.len()) == Some(&b'/'))
    }

    fn relative_components<'a>(&self, path: &'a Self) -> impl Iterator<Item = &'a str> {
        let offset = if self.is_root() { 1 } else { self.0.len() };
        path.0[offset..].split('/').filter(|p| !p.is_empty())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Regular,
    Directory,
    Symlink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Access(u8);

impl Access {
    pub const NONE: Self = Self(0);
    pub const EXECUTE: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const READ: Self = Self(4);
    pub const ALL: Self = Self(7);
    pub const fn bits(self) -> u8 {
        self.0
    }
    pub const fn contains(self, rhs: Self) -> bool {
        self.0 & rhs.0 == rhs.0
    }
    pub const fn union(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AclSubject {
    User(UserId),
    Group(GroupId),
    Everyone,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AclEntry {
    pub subject: AclSubject,
    pub allow: Access,
    pub deny: Access,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Metadata {
    pub inode: InodeId,
    pub kind: NodeKind,
    pub owner: UserId,
    pub group: GroupId,
    /// POSIX-like low nine permission bits (`0o777`).
    pub mode: u16,
    pub size: u64,
    pub generation: u32,
    /// All of these capabilities are required in addition to DAC permission.
    pub required_capabilities: u64,
    pub acl: Vec<AclEntry>,
}

impl Metadata {
    pub fn new(inode: InodeId, kind: NodeKind, owner: UserId, group: GroupId, mode: u16) -> Self {
        Self {
            inode,
            kind,
            owner,
            group,
            mode: mode & 0o777,
            size: 0,
            generation: 1,
            required_capabilities: 0,
            acl: Vec::new(),
        }
    }
}

pub mod capability {
    pub const DAC_OVERRIDE: u64 = 1 << 0;
    pub const CHOWN: u64 = 1 << 1;
    pub const MOUNT: u64 = 1 << 2;
    pub const QUOTA_ADMIN: u64 = 1 << 3;
    pub const SYSTEM: u64 = 1 << 63;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Credentials {
    pub user: UserId,
    pub primary_group: GroupId,
    pub supplementary_groups: Vec<GroupId>,
    pub capabilities: u64,
}

impl Credentials {
    pub fn user(user: UserId, group: GroupId) -> Self {
        Self {
            user,
            primary_group: group,
            supplementary_groups: Vec::new(),
            capabilities: 0,
        }
    }
    pub fn kernel() -> Self {
        Self {
            user: 0,
            primary_group: 0,
            supplementary_groups: Vec::new(),
            capabilities: u64::MAX,
        }
    }
    pub fn in_group(&self, group: GroupId) -> bool {
        self.primary_group == group || self.supplementary_groups.contains(&group)
    }
    pub fn has_capabilities(&self, caps: u64) -> bool {
        self.capabilities & caps == caps
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub primary_group: GroupId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub members: Vec<UserId>,
}

/// Identity registry used to construct trusted credentials. Persistent account
/// databases can serialize these records without coupling VFS to a disk format.
#[derive(Default)]
pub struct IdentityStore {
    users: BTreeMap<UserId, User>,
    groups: BTreeMap<GroupId, Group>,
}

impl IdentityStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_group(&mut self, group: Group) -> Result<()> {
        Self::validate_name(&group.name)?;
        if self.groups.contains_key(&group.id)
            || self.groups.values().any(|known| known.name == group.name)
        {
            return Err(VfsError::AlreadyExists);
        }
        self.groups.insert(group.id, group);
        Ok(())
    }

    pub fn add_user(&mut self, user: User) -> Result<()> {
        Self::validate_name(&user.name)?;
        if !self.groups.contains_key(&user.primary_group) {
            return Err(VfsError::NotFound);
        }
        if self.users.contains_key(&user.id)
            || self.users.values().any(|known| known.name == user.name)
        {
            return Err(VfsError::AlreadyExists);
        }
        self.users.insert(user.id, user);
        Ok(())
    }

    pub fn add_group_member(&mut self, group: GroupId, user: UserId) -> Result<()> {
        if !self.users.contains_key(&user) {
            return Err(VfsError::NotFound);
        }
        let members = &mut self
            .groups
            .get_mut(&group)
            .ok_or(VfsError::NotFound)?
            .members;
        if !members.contains(&user) {
            members.push(user);
        }
        Ok(())
    }

    pub fn user(&self, id: UserId) -> Option<&User> {
        self.users.get(&id)
    }

    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.get(&id)
    }

    pub fn credentials(&self, user: UserId, capabilities: u64) -> Result<Credentials> {
        let account = self.users.get(&user).ok_or(VfsError::NotFound)?;
        let supplementary_groups = self
            .groups
            .values()
            .filter(|group| group.id != account.primary_group && group.members.contains(&user))
            .map(|group| group.id)
            .collect();
        Ok(Credentials {
            user,
            primary_group: account.primary_group,
            supplementary_groups,
            capabilities,
        })
    }

    fn validate_name(name: &str) -> Result<()> {
        if name.is_empty()
            || name.len() > 64
            || name.as_bytes().contains(&0)
            || name.contains(['/', ':'])
        {
            Err(VfsError::InvalidArgument)
        } else {
            Ok(())
        }
    }
}

pub fn check_access(meta: &Metadata, credentials: &Credentials, requested: Access) -> Result<()> {
    if !credentials.has_capabilities(meta.required_capabilities) {
        return Err(VfsError::PermissionDenied);
    }
    if requested == Access::NONE || credentials.has_capabilities(capability::DAC_OVERRIDE) {
        return Ok(());
    }

    let matches = |subject: AclSubject| match subject {
        AclSubject::User(id) => credentials.user == id,
        AclSubject::Group(id) => credentials.in_group(id),
        AclSubject::Everyone => true,
    };
    let mut acl_allow = Access::NONE;
    for entry in meta.acl.iter().filter(|e| matches(e.subject)) {
        if entry.deny.bits() & requested.bits() != 0 {
            return Err(VfsError::PermissionDenied);
        }
        acl_allow = acl_allow.union(entry.allow);
    }

    let shift = if credentials.user == meta.owner {
        6
    } else if credentials.in_group(meta.group) {
        3
    } else {
        0
    };
    let mode_access = Access(((meta.mode >> shift) & 7) as u8);
    if mode_access.union(acl_allow).contains(requested) {
        Ok(())
    } else {
        Err(VfsError::PermissionDenied)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirEntry {
    pub name: String,
    pub inode: InodeId,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateNode {
    Regular,
    Directory,
    Symlink(String),
}

/// Storage-neutral inode backend. It receives only names relative to an inode;
/// path, security and mount policy remain in [`Vfs`].
pub trait Backend {
    fn root(&self) -> InodeId;
    fn lookup(&self, directory: InodeId, name: &str) -> Result<InodeId>;
    fn metadata(&self, inode: InodeId) -> Result<Metadata>;
    fn set_metadata(&mut self, metadata: &Metadata) -> Result<()>;
    fn create(
        &mut self,
        directory: InodeId,
        name: &str,
        node: CreateNode,
        owner: UserId,
        group: GroupId,
        mode: u16,
    ) -> Result<InodeId>;
    fn remove(&mut self, directory: InodeId, name: &str) -> Result<Metadata>;
    fn read(&self, inode: InodeId, offset: u64, output: &mut [u8]) -> Result<usize>;
    fn write(&mut self, inode: InodeId, offset: u64, input: &[u8]) -> Result<usize>;
    fn truncate(&mut self, inode: InodeId, size: u64) -> Result<()>;
    fn read_dir(&self, inode: InodeId) -> Result<Vec<DirEntry>>;
    fn read_link(&self, inode: InodeId) -> Result<String>;
    fn rename(
        &mut self,
        old_directory: InodeId,
        old_name: &str,
        new_directory: InodeId,
        new_name: &str,
    ) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenOptions(u8);

impl OpenOptions {
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const CREATE: Self = Self(4);
    pub const TRUNCATE: Self = Self(8);
    pub const APPEND: Self = Self(16);
    pub const fn new() -> Self {
        Self(0)
    }
    pub const fn read(mut self, yes: bool) -> Self {
        if yes {
            self.0 |= Self::READ.0
        }
        self
    }
    pub const fn write(mut self, yes: bool) -> Self {
        if yes {
            self.0 |= Self::WRITE.0
        }
        self
    }
    pub const fn create(mut self, yes: bool) -> Self {
        if yes {
            self.0 |= Self::CREATE.0
        }
        self
    }
    pub const fn truncate(mut self, yes: bool) -> Self {
        if yes {
            self.0 |= Self::TRUNCATE.0
        }
        self
    }
    pub const fn append(mut self, yes: bool) -> Self {
        if yes {
            self.0 |= Self::APPEND.0
        }
        self
    }
    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 != 0
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandleToken {
    pub slot: u32,
    pub generation: u32,
}

struct OpenHandle {
    mount: usize,
    inode: InodeId,
    inode_generation: u32,
    offset: u64,
    options: OpenOptions,
    credentials: Credentials,
}

struct HandleSlot {
    generation: u32,
    handle: Option<OpenHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Quota {
    pub byte_limit: u64,
    pub inode_limit: u64,
    pub bytes_used: u64,
    pub inodes_used: u64,
}

impl Quota {
    pub fn new(byte_limit: u64, inode_limit: u64) -> Self {
        Self {
            byte_limit,
            inode_limit,
            bytes_used: 0,
            inodes_used: 0,
        }
    }
}

struct Mount<B> {
    path: CanonicalPath,
    backend: B,
    read_only: bool,
}

pub struct Vfs<B: Backend> {
    mounts: Vec<Mount<B>>,
    handles: Vec<HandleSlot>,
    quotas: BTreeMap<UserId, Quota>,
    symlink_limit: usize,
}

impl<B: Backend> Vfs<B> {
    pub fn new(root: B) -> Self {
        Self {
            mounts: vec![Mount {
                path: CanonicalPath::root(),
                backend: root,
                read_only: false,
            }],
            handles: Vec::new(),
            quotas: BTreeMap::new(),
            symlink_limit: 32,
        }
    }

    pub fn mount(
        &mut self,
        path: &str,
        backend: B,
        read_only: bool,
        credentials: &Credentials,
    ) -> Result<()> {
        if !credentials.has_capabilities(capability::MOUNT) {
            return Err(VfsError::PermissionDenied);
        }
        let path = CanonicalPath::new(path)?;
        if path.is_root() || self.mounts.iter().any(|m| m.path == path) {
            return Err(VfsError::AlreadyExists);
        }
        let (resolved_mount, inode) = self.resolve(&path, true, credentials)?;
        if self.mounts[resolved_mount].backend.metadata(inode)?.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        self.mounts.push(Mount {
            path,
            backend,
            read_only,
        });
        Ok(())
    }

    pub fn unmount(&mut self, path: &str, credentials: &Credentials) -> Result<B> {
        if !credentials.has_capabilities(capability::MOUNT) {
            return Err(VfsError::PermissionDenied);
        }
        let path = CanonicalPath::new(path)?;
        let index = self
            .mounts
            .iter()
            .position(|m| m.path == path)
            .ok_or(VfsError::NotFound)?;
        if index == 0 {
            return Err(VfsError::InvalidArgument);
        }
        if self
            .handles
            .iter()
            .any(|s| s.handle.as_ref().is_some_and(|h| h.mount == index))
        {
            return Err(VfsError::ReadOnly);
        }
        let mount = self.mounts.remove(index);
        for slot in &mut self.handles {
            if let Some(handle) = &mut slot.handle {
                if handle.mount > index {
                    handle.mount -= 1;
                }
            }
        }
        Ok(mount.backend)
    }

    pub fn set_quota(
        &mut self,
        user: UserId,
        quota: Quota,
        credentials: &Credentials,
    ) -> Result<()> {
        if !credentials.has_capabilities(capability::QUOTA_ADMIN) {
            return Err(VfsError::PermissionDenied);
        }
        if quota.bytes_used > quota.byte_limit || quota.inodes_used > quota.inode_limit {
            return Err(VfsError::InvalidArgument);
        }
        self.quotas.insert(user, quota);
        Ok(())
    }

    pub fn quota(&self, user: UserId) -> Option<Quota> {
        self.quotas.get(&user).copied()
    }

    pub fn metadata(&self, path: &str, credentials: &Credentials) -> Result<Metadata> {
        let path = CanonicalPath::new(path)?;
        let (mount, inode) = self.resolve(&path, true, credentials)?;
        let meta = self.mounts[mount].backend.metadata(inode)?;
        check_access(&meta, credentials, Access::READ)?;
        Ok(meta)
    }

    pub fn set_metadata(
        &mut self,
        path: &str,
        mut updated: Metadata,
        credentials: &Credentials,
    ) -> Result<()> {
        let path = CanonicalPath::new(path)?;
        let (mount, inode) = self.resolve(&path, true, credentials)?;
        if self.mounts[mount].read_only {
            return Err(VfsError::ReadOnly);
        }
        let current = self.mounts[mount].backend.metadata(inode)?;
        if current.owner != credentials.user && !credentials.has_capabilities(capability::CHOWN) {
            return Err(VfsError::PermissionDenied);
        }
        updated.inode = inode;
        updated.kind = current.kind;
        updated.size = current.size;
        updated.generation = current.generation;
        self.mounts[mount].backend.set_metadata(&updated)
    }

    pub fn create_dir(&mut self, path: &str, mode: u16, credentials: &Credentials) -> Result<()> {
        self.create_node(path, CreateNode::Directory, mode, credentials)
            .map(|_| ())
    }

    pub fn symlink(&mut self, target: &str, path: &str, credentials: &Credentials) -> Result<()> {
        if target.as_bytes().contains(&0) {
            return Err(VfsError::InvalidPath);
        }
        self.create_node(
            path,
            CreateNode::Symlink(target.to_string()),
            0o777,
            credentials,
        )
        .map(|_| ())
    }

    pub fn remove(&mut self, path: &str, credentials: &Credentials) -> Result<()> {
        let path = CanonicalPath::new(path)?;
        let (mount, parent, name) = self.resolve_parent(&path, credentials)?;
        self.require_parent_write(mount, parent, credentials)?;
        if self.mounts[mount].read_only {
            return Err(VfsError::ReadOnly);
        }
        let inode = self.mounts[mount].backend.lookup(parent, name)?;
        let meta = self.mounts[mount].backend.metadata(inode)?;
        let removed = self.mounts[mount].backend.remove(parent, name)?;
        debug_assert_eq!(removed.inode, meta.inode);
        self.release_quota(removed.owner, removed.size, 1);
        Ok(())
    }

    pub fn read_dir(&self, path: &str, credentials: &Credentials) -> Result<Vec<DirEntry>> {
        let path = CanonicalPath::new(path)?;
        let (mount, inode) = self.resolve(&path, true, credentials)?;
        let meta = self.mounts[mount].backend.metadata(inode)?;
        if meta.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        check_access(&meta, credentials, Access::READ.union(Access::EXECUTE))?;
        self.mounts[mount].backend.read_dir(inode)
    }

    pub fn rename(&mut self, old: &str, new: &str, credentials: &Credentials) -> Result<()> {
        let old = CanonicalPath::new(old)?;
        let new = CanonicalPath::new(new)?;
        let (old_mount, old_parent, old_name) = self.resolve_parent(&old, credentials)?;
        let (new_mount, new_parent, new_name) = self.resolve_parent(&new, credentials)?;
        if old_mount != new_mount {
            return Err(VfsError::CrossDevice);
        }
        if self.mounts[old_mount].read_only {
            return Err(VfsError::ReadOnly);
        }
        self.require_parent_write(old_mount, old_parent, credentials)?;
        self.require_parent_write(new_mount, new_parent, credentials)?;
        let moved = self.mounts[old_mount]
            .backend
            .lookup(old_parent, old_name)?;
        if self.mounts[old_mount].backend.metadata(moved)?.kind == NodeKind::Directory {
            let new_parent_path = new.parent().ok_or(VfsError::InvalidArgument)?;
            if old.is_prefix_of(&new_parent_path) {
                return Err(VfsError::InvalidArgument);
            }
        }
        self.mounts[old_mount]
            .backend
            .rename(old_parent, old_name, new_parent, new_name)
    }

    pub fn open(
        &mut self,
        path: &str,
        options: OpenOptions,
        credentials: &Credentials,
    ) -> Result<HandleToken> {
        if !options.contains(OpenOptions::READ) && !options.contains(OpenOptions::WRITE) {
            return Err(VfsError::InvalidArgument);
        }
        if (options.contains(OpenOptions::CREATE) || options.contains(OpenOptions::TRUNCATE))
            && !options.contains(OpenOptions::WRITE)
        {
            return Err(VfsError::InvalidArgument);
        }
        let path = CanonicalPath::new(path)?;
        let resolved = self.resolve(&path, true, credentials);
        let (mount, inode) = match resolved {
            Ok(pair) => pair,
            Err(VfsError::NotFound) if options.contains(OpenOptions::CREATE) => {
                self.create_node_canonical(&path, CreateNode::Regular, 0o660, credentials)?
            }
            Err(error) => return Err(error),
        };
        let mut meta = self.mounts[mount].backend.metadata(inode)?;
        if meta.kind == NodeKind::Directory {
            return Err(VfsError::IsDirectory);
        }
        let requested = (if options.contains(OpenOptions::READ) {
            Access::READ
        } else {
            Access::NONE
        })
        .union(if options.contains(OpenOptions::WRITE) {
            Access::WRITE
        } else {
            Access::NONE
        });
        check_access(&meta, credentials, requested)?;
        if options.contains(OpenOptions::WRITE) && self.mounts[mount].read_only {
            return Err(VfsError::ReadOnly);
        }
        if options.contains(OpenOptions::TRUNCATE) {
            self.adjust_quota(meta.owner, -(meta.size as i128), 0)?;
            if let Err(error) = self.mounts[mount].backend.truncate(inode, 0) {
                self.adjust_quota(meta.owner, meta.size as i128, 0).ok();
                return Err(error);
            }
            meta.size = 0;
        }
        let offset = if options.contains(OpenOptions::APPEND) {
            meta.size
        } else {
            0
        };
        let handle = OpenHandle {
            mount,
            inode,
            inode_generation: meta.generation,
            offset,
            options,
            credentials: credentials.clone(),
        };
        Ok(self.allocate_handle(handle))
    }

    pub fn close(&mut self, token: HandleToken) -> Result<()> {
        let slot = self
            .handles
            .get_mut(token.slot as usize)
            .ok_or(VfsError::InvalidHandle)?;
        if slot.generation != token.generation || slot.handle.is_none() {
            return Err(VfsError::InvalidHandle);
        }
        slot.handle = None;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        Ok(())
    }

    pub fn seek(&mut self, token: HandleToken, offset: u64) -> Result<()> {
        self.handle_mut(token)?.offset = offset;
        Ok(())
    }

    pub fn read(&mut self, token: HandleToken, output: &mut [u8]) -> Result<usize> {
        let index = self.validate_handle(token)?;
        let (mount, inode, offset, can_read, credentials) = {
            let handle = self.handles[index].handle.as_ref().unwrap();
            (
                handle.mount,
                handle.inode,
                handle.offset,
                handle.options.contains(OpenOptions::READ),
                handle.credentials.clone(),
            )
        };
        if !can_read {
            return Err(VfsError::PermissionDenied);
        }
        check_access(
            &self.mounts[mount].backend.metadata(inode)?,
            &credentials,
            Access::READ,
        )?;
        let count = self.mounts[mount].backend.read(inode, offset, output)?;
        self.handles[index].handle.as_mut().unwrap().offset = offset.saturating_add(count as u64);
        Ok(count)
    }

    pub fn write(&mut self, token: HandleToken, input: &[u8]) -> Result<usize> {
        let index = self.validate_handle(token)?;
        let (mount, inode, mut offset, options, owner, credentials) = {
            let handle = self.handles[index].handle.as_ref().unwrap();
            let meta = self.mounts[handle.mount].backend.metadata(handle.inode)?;
            (
                handle.mount,
                handle.inode,
                handle.offset,
                handle.options,
                meta.owner,
                handle.credentials.clone(),
            )
        };
        if !options.contains(OpenOptions::WRITE) {
            return Err(VfsError::PermissionDenied);
        }
        check_access(
            &self.mounts[mount].backend.metadata(inode)?,
            &credentials,
            Access::WRITE,
        )?;
        if self.mounts[mount].read_only {
            return Err(VfsError::ReadOnly);
        }
        let old_size = self.mounts[mount].backend.metadata(inode)?.size;
        if options.contains(OpenOptions::APPEND) {
            offset = old_size;
        }
        let requested_end = offset
            .checked_add(input.len() as u64)
            .ok_or(VfsError::NoSpace)?;
        let growth = requested_end.saturating_sub(old_size);
        self.adjust_quota(owner, growth as i128, 0)?;
        let count = match self.mounts[mount].backend.write(inode, offset, input) {
            Ok(count) => count,
            Err(error) => {
                self.adjust_quota(owner, -(growth as i128), 0).ok();
                return Err(error);
            }
        };
        let actual_end = offset.saturating_add(count as u64);
        let actual_growth = actual_end.saturating_sub(old_size);
        if actual_growth < growth {
            self.adjust_quota(owner, -((growth - actual_growth) as i128), 0)
                .ok();
        }
        self.handles[index].handle.as_mut().unwrap().offset = actual_end;
        Ok(count)
    }

    pub fn truncate(&mut self, token: HandleToken, size: u64) -> Result<()> {
        let index = self.validate_handle(token)?;
        let (mount, inode, can_write, credentials) = {
            let h = self.handles[index].handle.as_ref().unwrap();
            (
                h.mount,
                h.inode,
                h.options.contains(OpenOptions::WRITE),
                h.credentials.clone(),
            )
        };
        if !can_write {
            return Err(VfsError::PermissionDenied);
        }
        let meta = self.mounts[mount].backend.metadata(inode)?;
        check_access(&meta, &credentials, Access::WRITE)?;
        let delta = size as i128 - meta.size as i128;
        self.adjust_quota(meta.owner, delta, 0)?;
        if let Err(error) = self.mounts[mount].backend.truncate(inode, size) {
            self.adjust_quota(meta.owner, -delta, 0).ok();
            return Err(error);
        }
        Ok(())
    }

    fn find_mount(&self, path: &CanonicalPath) -> usize {
        self.mounts
            .iter()
            .enumerate()
            .filter(|(_, m)| m.path.is_prefix_of(path))
            .max_by_key(|(_, m)| m.path.as_str().len())
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn resolve(
        &self,
        initial: &CanonicalPath,
        follow_final: bool,
        credentials: &Credentials,
    ) -> Result<(usize, InodeId)> {
        let mut path = initial.clone();
        let mut seen: Vec<(usize, InodeId)> = Vec::new();
        'restart: loop {
            let mount = self.find_mount(&path);
            let root = self.mounts[mount].backend.root();
            let components: Vec<&str> =
                self.mounts[mount].path.relative_components(&path).collect();
            let mut inode = root;
            let mut parent_path = self.mounts[mount].path.clone();
            for (index, name) in components.iter().enumerate() {
                let directory = self.mounts[mount].backend.metadata(inode)?;
                if directory.kind != NodeKind::Directory {
                    return Err(VfsError::NotDirectory);
                }
                check_access(&directory, credentials, Access::EXECUTE)?;
                let next = self.mounts[mount].backend.lookup(inode, name)?;
                let meta = self.mounts[mount].backend.metadata(next)?;
                let is_final = index + 1 == components.len();
                if meta.kind == NodeKind::Symlink && (follow_final || !is_final) {
                    if seen.len() >= self.symlink_limit || seen.contains(&(mount, next)) {
                        return Err(VfsError::TooManySymlinks);
                    }
                    seen.push((mount, next));
                    let target = self.mounts[mount].backend.read_link(next)?;
                    let mut expanded = if target.starts_with('/') {
                        CanonicalPath::new(&target)?
                    } else {
                        parent_path.join(&target)?
                    };
                    for rest in &components[index + 1..] {
                        expanded = expanded.join(rest)?;
                    }
                    path = expanded;
                    continue 'restart;
                }
                inode = next;
                if !is_final && meta.kind != NodeKind::Directory {
                    return Err(VfsError::NotDirectory);
                }
                parent_path = parent_path.join(name)?;
            }
            return Ok((mount, inode));
        }
    }

    fn resolve_parent<'a>(
        &self,
        path: &'a CanonicalPath,
        credentials: &Credentials,
    ) -> Result<(usize, InodeId, &'a str)> {
        let parent_path = path.parent().ok_or(VfsError::InvalidArgument)?;
        let name = path.file_name().ok_or(VfsError::InvalidArgument)?;
        let (mount, parent) = self.resolve(&parent_path, true, credentials)?;
        let meta = self.mounts[mount].backend.metadata(parent)?;
        if meta.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        Ok((mount, parent, name))
    }

    fn require_parent_write(
        &self,
        mount: usize,
        parent: InodeId,
        credentials: &Credentials,
    ) -> Result<()> {
        let meta = self.mounts[mount].backend.metadata(parent)?;
        check_access(&meta, credentials, Access::WRITE.union(Access::EXECUTE))
    }

    fn create_node(
        &mut self,
        path: &str,
        node: CreateNode,
        mode: u16,
        credentials: &Credentials,
    ) -> Result<(usize, InodeId)> {
        let path = CanonicalPath::new(path)?;
        self.create_node_canonical(&path, node, mode, credentials)
    }

    fn create_node_canonical(
        &mut self,
        path: &CanonicalPath,
        node: CreateNode,
        mode: u16,
        credentials: &Credentials,
    ) -> Result<(usize, InodeId)> {
        let (mount, parent, name) = self.resolve_parent(path, credentials)?;
        self.require_parent_write(mount, parent, credentials)?;
        if self.mounts[mount].read_only {
            return Err(VfsError::ReadOnly);
        }
        let initial_bytes = match &node {
            CreateNode::Symlink(target) => target.len() as i128,
            _ => 0,
        };
        self.adjust_quota(credentials.user, initial_bytes, 1)?;
        match self.mounts[mount].backend.create(
            parent,
            name,
            node,
            credentials.user,
            credentials.primary_group,
            mode,
        ) {
            Ok(inode) => Ok((mount, inode)),
            Err(error) => {
                self.adjust_quota(credentials.user, -initial_bytes, -1).ok();
                Err(error)
            }
        }
    }

    fn allocate_handle(&mut self, handle: OpenHandle) -> HandleToken {
        if let Some((index, slot)) = self
            .handles
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.handle.is_none())
        {
            slot.handle = Some(handle);
            return HandleToken {
                slot: index as u32,
                generation: slot.generation,
            };
        }
        self.handles.push(HandleSlot {
            generation: 1,
            handle: Some(handle),
        });
        HandleToken {
            slot: (self.handles.len() - 1) as u32,
            generation: 1,
        }
    }

    fn validate_handle(&self, token: HandleToken) -> Result<usize> {
        let index = token.slot as usize;
        let slot = self.handles.get(index).ok_or(VfsError::InvalidHandle)?;
        let handle = slot
            .handle
            .as_ref()
            .filter(|_| slot.generation == token.generation)
            .ok_or(VfsError::InvalidHandle)?;
        let current = self
            .mounts
            .get(handle.mount)
            .ok_or(VfsError::InvalidHandle)?
            .backend
            .metadata(handle.inode)?;
        if current.generation != handle.inode_generation {
            return Err(VfsError::InvalidHandle);
        }
        Ok(index)
    }

    fn handle_mut(&mut self, token: HandleToken) -> Result<&mut OpenHandle> {
        let index = self.validate_handle(token)?;
        Ok(self.handles[index].handle.as_mut().unwrap())
    }

    fn adjust_quota(&mut self, user: UserId, bytes: i128, inodes: i64) -> Result<()> {
        let Some(quota) = self.quotas.get_mut(&user) else {
            return Ok(());
        };
        let new_bytes = if bytes >= 0 {
            quota.bytes_used.checked_add(bytes as u64)
        } else {
            quota.bytes_used.checked_sub((-bytes) as u64)
        }
        .ok_or(VfsError::Corrupt)?;
        let new_inodes = if inodes >= 0 {
            quota.inodes_used.checked_add(inodes as u64)
        } else {
            quota.inodes_used.checked_sub((-inodes) as u64)
        }
        .ok_or(VfsError::Corrupt)?;
        if new_bytes > quota.byte_limit || new_inodes > quota.inode_limit {
            return Err(VfsError::QuotaExceeded);
        }
        quota.bytes_used = new_bytes;
        quota.inodes_used = new_inodes;
        Ok(())
    }

    fn release_quota(&mut self, user: UserId, bytes: u64, inodes: u64) {
        if let Some(quota) = self.quotas.get_mut(&user) {
            quota.bytes_used = quota.bytes_used.saturating_sub(bytes);
            quota.inodes_used = quota.inodes_used.saturating_sub(inodes);
        }
    }
}

/// Small allocation-backed backend, useful for the initial RAM filesystem,
/// recovery environments and deterministic tests.
pub struct MemBackend {
    nodes: BTreeMap<InodeId, MemNode>,
    next_inode: InodeId,
}

struct MemNode {
    metadata: Metadata,
    payload: MemPayload,
}

enum MemPayload {
    File(Vec<u8>),
    Directory(BTreeMap<String, InodeId>),
    Symlink(String),
}

impl MemBackend {
    pub fn new(owner: UserId, group: GroupId, mode: u16) -> Self {
        let root = MemNode {
            metadata: Metadata::new(1, NodeKind::Directory, owner, group, mode),
            payload: MemPayload::Directory(BTreeMap::new()),
        };
        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        Self {
            nodes,
            next_inode: 2,
        }
    }

    fn node(&self, inode: InodeId) -> Result<&MemNode> {
        self.nodes.get(&inode).ok_or(VfsError::NotFound)
    }
    fn node_mut(&mut self, inode: InodeId) -> Result<&mut MemNode> {
        self.nodes.get_mut(&inode).ok_or(VfsError::NotFound)
    }
    fn valid_name(name: &str) -> Result<()> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.as_bytes().contains(&0)
        {
            Err(VfsError::InvalidPath)
        } else if name.len() > 255 {
            Err(VfsError::NameTooLong)
        } else {
            Ok(())
        }
    }
}

impl Default for MemBackend {
    fn default() -> Self {
        Self::new(0, 0, 0o755)
    }
}

impl Backend for MemBackend {
    fn root(&self) -> InodeId {
        1
    }
    fn lookup(&self, directory: InodeId, name: &str) -> Result<InodeId> {
        match &self.node(directory)?.payload {
            MemPayload::Directory(entries) => entries.get(name).copied().ok_or(VfsError::NotFound),
            _ => Err(VfsError::NotDirectory),
        }
    }
    fn metadata(&self, inode: InodeId) -> Result<Metadata> {
        Ok(self.node(inode)?.metadata.clone())
    }
    fn set_metadata(&mut self, metadata: &Metadata) -> Result<()> {
        let node = self.node_mut(metadata.inode)?;
        if node.metadata.generation != metadata.generation || node.metadata.kind != metadata.kind {
            return Err(VfsError::InvalidArgument);
        }
        node.metadata = metadata.clone();
        Ok(())
    }
    fn create(
        &mut self,
        directory: InodeId,
        name: &str,
        node: CreateNode,
        owner: UserId,
        group: GroupId,
        mode: u16,
    ) -> Result<InodeId> {
        Self::valid_name(name)?;
        match &self.node(directory)?.payload {
            MemPayload::Directory(entries) if entries.contains_key(name) => {
                return Err(VfsError::AlreadyExists)
            }
            MemPayload::Directory(_) => {}
            _ => return Err(VfsError::NotDirectory),
        }
        let inode = self.next_inode;
        self.next_inode = self.next_inode.checked_add(1).ok_or(VfsError::NoSpace)?;
        let (kind, payload, size) = match node {
            CreateNode::Regular => (NodeKind::Regular, MemPayload::File(Vec::new()), 0),
            CreateNode::Directory => (
                NodeKind::Directory,
                MemPayload::Directory(BTreeMap::new()),
                0,
            ),
            CreateNode::Symlink(target) => {
                let size = target.len() as u64;
                (NodeKind::Symlink, MemPayload::Symlink(target), size)
            }
        };
        let mut metadata = Metadata::new(inode, kind, owner, group, mode);
        metadata.size = size;
        self.nodes.insert(inode, MemNode { metadata, payload });
        match &mut self.node_mut(directory)?.payload {
            MemPayload::Directory(entries) => {
                entries.insert(name.to_string(), inode);
            }
            _ => unreachable!(),
        }
        Ok(inode)
    }
    fn remove(&mut self, directory: InodeId, name: &str) -> Result<Metadata> {
        let inode = self.lookup(directory, name)?;
        if let MemPayload::Directory(entries) = &self.node(inode)?.payload {
            if !entries.is_empty() {
                return Err(VfsError::DirectoryNotEmpty);
            }
        }
        match &mut self.node_mut(directory)?.payload {
            MemPayload::Directory(entries) => {
                entries.remove(name);
            }
            _ => return Err(VfsError::NotDirectory),
        }
        Ok(self.nodes.remove(&inode).ok_or(VfsError::Corrupt)?.metadata)
    }
    fn read(&self, inode: InodeId, offset: u64, output: &mut [u8]) -> Result<usize> {
        let MemPayload::File(data) = &self.node(inode)?.payload else {
            return Err(VfsError::IsDirectory);
        };
        let offset = cmp::min(offset, data.len() as u64) as usize;
        let count = cmp::min(output.len(), data.len() - offset);
        output[..count].copy_from_slice(&data[offset..offset + count]);
        Ok(count)
    }
    fn write(&mut self, inode: InodeId, offset: u64, input: &[u8]) -> Result<usize> {
        let end = offset
            .checked_add(input.len() as u64)
            .ok_or(VfsError::NoSpace)?;
        let end_usize = usize::try_from(end).map_err(|_| VfsError::NoSpace)?;
        let node = self.node_mut(inode)?;
        let MemPayload::File(data) = &mut node.payload else {
            return Err(VfsError::IsDirectory);
        };
        if data.len() < end_usize {
            data.resize(end_usize, 0);
        }
        data[offset as usize..end_usize].copy_from_slice(input);
        node.metadata.size = data.len() as u64;
        Ok(input.len())
    }
    fn truncate(&mut self, inode: InodeId, size: u64) -> Result<()> {
        let size = usize::try_from(size).map_err(|_| VfsError::NoSpace)?;
        let node = self.node_mut(inode)?;
        let MemPayload::File(data) = &mut node.payload else {
            return Err(VfsError::IsDirectory);
        };
        data.resize(size, 0);
        node.metadata.size = size as u64;
        Ok(())
    }
    fn read_dir(&self, inode: InodeId) -> Result<Vec<DirEntry>> {
        let MemPayload::Directory(entries) = &self.node(inode)?.payload else {
            return Err(VfsError::NotDirectory);
        };
        entries
            .iter()
            .map(|(name, inode)| {
                let node = self.node(*inode)?;
                Ok(DirEntry {
                    name: name.clone(),
                    inode: *inode,
                    kind: node.metadata.kind,
                })
            })
            .collect()
    }
    fn read_link(&self, inode: InodeId) -> Result<String> {
        match &self.node(inode)?.payload {
            MemPayload::Symlink(target) => Ok(target.clone()),
            _ => Err(VfsError::InvalidArgument),
        }
    }
    fn rename(
        &mut self,
        old_directory: InodeId,
        old_name: &str,
        new_directory: InodeId,
        new_name: &str,
    ) -> Result<()> {
        Self::valid_name(new_name)?;
        let inode = self.lookup(old_directory, old_name)?;
        if self.lookup(new_directory, new_name).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        if self.node(new_directory)?.metadata.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        if inode == new_directory {
            return Err(VfsError::InvalidArgument);
        }
        if let MemPayload::Directory(entries) = &mut self.node_mut(old_directory)?.payload {
            entries.remove(old_name);
        }
        if let MemPayload::Directory(entries) = &mut self.node_mut(new_directory)?.payload {
            entries.insert(new_name.to_string(), inode);
        }
        Ok(())
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod security_tests {
    use super::*;

    fn admin() -> Credentials {
        Credentials::kernel()
    }
    fn user() -> Credentials {
        Credentials::user(1000, 100)
    }
    fn vfs() -> Vfs<MemBackend> {
        let mut fs = Vfs::new(MemBackend::new(0, 0, 0o777));
        fs.create_dir("/Домівка", 0o777, &admin()).unwrap();
        fs
    }

    #[test]
    fn ukrainian_users_and_groups_build_credentials() {
        let mut identities = IdentityStore::new();
        identities
            .add_group(Group {
                id: 100,
                name: "користувачі".into(),
                members: Vec::new(),
            })
            .unwrap();
        identities
            .add_group(Group {
                id: 200,
                name: "розробники".into(),
                members: Vec::new(),
            })
            .unwrap();
        identities
            .add_user(User {
                id: 1000,
                name: "Олена".into(),
                primary_group: 100,
            })
            .unwrap();
        identities.add_group_member(200, 1000).unwrap();
        let credentials = identities.credentials(1000, capability::SYSTEM).unwrap();
        assert_eq!(credentials.primary_group, 100);
        assert!(credentials.in_group(200));
        assert!(credentials.has_capabilities(capability::SYSTEM));
        assert_eq!(identities.user(1000).unwrap().name, "Олена");
    }

    #[test]
    fn directory_search_permission_cannot_be_bypassed() {
        let mut fs = vfs();
        fs.create_dir("/Домівка/Приватно", 0o700, &user()).unwrap();
        let path = "/Домівка/Приватно/публічна-назва.txt";
        let handle = fs
            .open(
                path,
                OpenOptions::new().read(true).write(true).create(true),
                &user(),
            )
            .unwrap();
        fs.close(handle).unwrap();
        let mut metadata = fs.metadata(path, &user()).unwrap();
        metadata.mode = 0o644;
        fs.set_metadata(path, metadata, &user()).unwrap();
        assert_eq!(
            fs.open(
                path,
                OpenOptions::new().read(true),
                &Credentials::user(2000, 200)
            ),
            Err(VfsError::PermissionDenied)
        );
    }

    #[test]
    fn acl_revocation_applies_to_an_existing_handle() {
        let mut fs = vfs();
        let path = "/Домівка/звіт";
        let owner_handle = fs
            .open(
                path,
                OpenOptions::new().read(true).write(true).create(true),
                &user(),
            )
            .unwrap();
        fs.write(owner_handle, b"report").unwrap();
        let guest = Credentials::user(2000, 200);
        let mut metadata = fs.metadata(path, &user()).unwrap();
        metadata.acl.push(AclEntry {
            subject: AclSubject::User(2000),
            allow: Access::READ,
            deny: Access::NONE,
        });
        fs.set_metadata(path, metadata.clone(), &user()).unwrap();
        let guest_handle = fs
            .open(path, OpenOptions::new().read(true), &guest)
            .unwrap();
        metadata.acl.push(AclEntry {
            subject: AclSubject::User(2000),
            allow: Access::NONE,
            deny: Access::READ,
        });
        fs.set_metadata(path, metadata, &user()).unwrap();
        let mut output = [0; 8];
        assert_eq!(
            fs.read(guest_handle, &mut output),
            Err(VfsError::PermissionDenied)
        );
    }

    #[test]
    fn symlink_payload_is_charged_to_quota_and_rolled_back() {
        let mut fs = vfs();
        fs.set_quota(1000, Quota::new(4, 1), &admin()).unwrap();
        assert_eq!(
            fs.symlink("12345", "/Домівка/довге", &user()),
            Err(VfsError::QuotaExceeded)
        );
        assert_eq!(fs.quota(1000).unwrap().inodes_used, 0);
        fs.symlink("1234", "/Домівка/коротке", &user()).unwrap();
        assert_eq!(fs.quota(1000).unwrap().bytes_used, 4);
        fs.remove("/Домівка/коротке", &user()).unwrap();
        assert_eq!(fs.quota(1000).unwrap(), Quota::new(4, 1));
    }

    #[test]
    fn directory_cannot_be_renamed_into_its_descendant() {
        let mut fs = vfs();
        fs.create_dir("/Домівка/А", 0o770, &user()).unwrap();
        fs.create_dir("/Домівка/А/Б", 0o770, &user()).unwrap();
        assert_eq!(
            fs.rename("/Домівка/А", "/Домівка/А/Б/А", &user()),
            Err(VfsError::InvalidArgument)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin() -> Credentials {
        Credentials::kernel()
    }
    fn user() -> Credentials {
        Credentials::user(1000, 100)
    }
    fn vfs() -> Vfs<MemBackend> {
        let mut fs = Vfs::new(MemBackend::new(0, 0, 0o777));
        fs.create_dir("/Домівка", 0o777, &admin()).unwrap();
        fs
    }

    #[test]
    fn canonical_utf8_paths_are_normalized_and_confined() {
        assert_eq!(
            CanonicalPath::new("/Документи//./Проєкт/../Нотатки")
                .unwrap()
                .as_str(),
            "/Документи/Нотатки"
        );
        assert_eq!(CanonicalPath::new("/../../etc"), Err(VfsError::InvalidPath));
        assert_eq!(CanonicalPath::new("relative"), Err(VfsError::InvalidPath));
        assert_eq!(CanonicalPath::new("/bad\0name"), Err(VfsError::InvalidPath));
    }

    #[test]
    fn create_write_read_truncate_append_and_stale_handle() {
        let mut fs = vfs();
        let options = OpenOptions::new().read(true).write(true).create(true);
        let h = fs.open("/Домівка/план.txt", options, &user()).unwrap();
        fs.write(h, "Nova ОС".as_bytes()).unwrap();
        fs.seek(h, 0).unwrap();
        let mut out = [0; 32];
        let n = fs.read(h, &mut out).unwrap();
        assert_eq!(&out[..n], "Nova ОС".as_bytes());
        fs.truncate(h, 4).unwrap();
        fs.close(h).unwrap();
        assert_eq!(fs.read(h, &mut out), Err(VfsError::InvalidHandle));
        let a = fs
            .open(
                "/Домівка/план.txt",
                OpenOptions::new().write(true).append(true),
                &user(),
            )
            .unwrap();
        fs.write(a, b"!").unwrap();
        let r = fs
            .open("/Домівка/план.txt", OpenOptions::new().read(true), &user())
            .unwrap();
        let n = fs.read(r, &mut out).unwrap();
        assert_eq!(&out[..n], b"Nova!");
    }

    #[test]
    fn posix_owner_group_acl_deny_and_required_capability() {
        let mut fs = vfs();
        let h = fs
            .open(
                "/Домівка/секрет",
                OpenOptions::new().write(true).create(true),
                &user(),
            )
            .unwrap();
        fs.write(h, b"secret").unwrap();
        fs.close(h).unwrap();
        let mut meta = fs.metadata("/Домівка/секрет", &user()).unwrap();
        meta.mode = 0o640;
        meta.acl.push(AclEntry {
            subject: AclSubject::User(2000),
            allow: Access::READ,
            deny: Access::NONE,
        });
        fs.set_metadata("/Домівка/секрет", meta.clone(), &user())
            .unwrap();
        let guest = Credentials::user(2000, 200);
        assert!(fs
            .open("/Домівка/секрет", OpenOptions::new().read(true), &guest)
            .is_ok());
        meta.acl.push(AclEntry {
            subject: AclSubject::User(2000),
            allow: Access::NONE,
            deny: Access::READ,
        });
        meta.required_capabilities = capability::SYSTEM;
        fs.set_metadata("/Домівка/секрет", meta, &user()).unwrap();
        assert_eq!(
            fs.open("/Домівка/секрет", OpenOptions::new().read(true), &guest),
            Err(VfsError::PermissionDenied)
        );
        assert!(fs
            .open("/Домівка/секрет", OpenOptions::new().read(true), &admin())
            .is_ok());
    }

    #[test]
    fn symlinks_resolve_relative_absolute_and_detect_loops() {
        let mut fs = vfs();
        let h = fs
            .open(
                "/Домівка/файл",
                OpenOptions::new().write(true).create(true),
                &user(),
            )
            .unwrap();
        fs.write(h, b"ok").unwrap();
        fs.symlink("файл", "/Домівка/посилання", &user()).unwrap();
        assert!(fs
            .open("/Домівка/посилання", OpenOptions::new().read(true), &user())
            .is_ok());
        fs.symlink("/Домівка/б", "/Домівка/а", &user()).unwrap();
        fs.symlink("/Домівка/а", "/Домівка/б", &user()).unwrap();
        assert_eq!(
            fs.open("/Домівка/а", OpenOptions::new().read(true), &user()),
            Err(VfsError::TooManySymlinks)
        );
    }

    #[test]
    fn directory_iteration_and_atomic_rename() {
        let mut fs = vfs();
        fs.create_dir("/Домівка/Робота", 0o770, &user()).unwrap();
        fs.open(
            "/Домівка/Робота/чернетка.md",
            OpenOptions::new().write(true).create(true),
            &user(),
        )
        .unwrap();
        fs.rename(
            "/Домівка/Робота/чернетка.md",
            "/Домівка/Робота/готово.md",
            &user(),
        )
        .unwrap();
        let entries = fs.read_dir("/Домівка/Робота", &user()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "готово.md");
    }

    #[test]
    fn quotas_are_enforced_and_released() {
        let mut fs = vfs();
        fs.set_quota(1000, Quota::new(5, 1), &admin()).unwrap();
        let h = fs
            .open(
                "/Домівка/q",
                OpenOptions::new().write(true).create(true),
                &user(),
            )
            .unwrap();
        assert_eq!(fs.write(h, b"123456"), Err(VfsError::QuotaExceeded));
        fs.write(h, b"12345").unwrap();
        assert_eq!(
            fs.open(
                "/Домівка/q2",
                OpenOptions::new().write(true).create(true),
                &user()
            ),
            Err(VfsError::QuotaExceeded)
        );
        fs.close(h).unwrap();
        fs.remove("/Домівка/q", &user()).unwrap();
        assert_eq!(fs.quota(1000).unwrap().bytes_used, 0);
        assert!(fs
            .open(
                "/Домівка/q2",
                OpenOptions::new().write(true).create(true),
                &user()
            )
            .is_ok());
    }

    #[test]
    fn mount_selection_read_only_and_cross_device_rules() {
        let mut fs = vfs();
        fs.create_dir("/mnt", 0o777, &admin()).unwrap();
        fs.mount("/mnt", MemBackend::new(0, 0, 0o777), true, &admin())
            .unwrap();
        assert_eq!(
            fs.open(
                "/mnt/x",
                OpenOptions::new().write(true).create(true),
                &user()
            ),
            Err(VfsError::ReadOnly)
        );
        let h = fs
            .open(
                "/Домівка/x",
                OpenOptions::new().write(true).create(true),
                &user(),
            )
            .unwrap();
        fs.close(h).unwrap();
        assert_eq!(
            fs.rename("/Домівка/x", "/mnt/x", &user()),
            Err(VfsError::CrossDevice)
        );
    }

    #[test]
    fn permissions_block_unrelated_users_and_allow_supplementary_group() {
        let mut fs = vfs();
        let h = fs
            .open(
                "/Домівка/group",
                OpenOptions::new().write(true).create(true),
                &user(),
            )
            .unwrap();
        fs.close(h).unwrap();
        let stranger = Credentials::user(2000, 200);
        assert_eq!(
            fs.open("/Домівка/group", OpenOptions::new().read(true), &stranger),
            Err(VfsError::PermissionDenied)
        );
        let mut teammate = stranger;
        teammate.supplementary_groups.push(100);
        assert!(fs
            .open("/Домівка/group", OpenOptions::new().read(true), &teammate)
            .is_ok());
    }
}
