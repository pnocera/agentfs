//! Windows HostFS implementation using path-backed operations.
//!
//! Windows does not expose Unix-style inode numbers, so this implementation
//! maps Windows file identities to stable SDK inode numbers. It prefers
//! `FileIdInfo` and falls back to `BY_HANDLE_FILE_INFORMATION`.

use super::{
    BoxedFile, DirEntry, File, FileSystem, FilesystemStats, FsError, Stats, TimeChange,
    OPEN_READONLY, OPEN_READWRITE, OPEN_WRITEONLY, S_IFDIR, S_IFLNK, S_IFREG,
};
use crate::error::{Error, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::fs::{self, File as StdFile, Metadata, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetDiskFreeSpaceExW, GetFileInformationByHandle, GetFileInformationByHandleEx,
    SetFileTime, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
};

/// Root inode number (matches FUSE convention).
pub const ROOT_INO: i64 = 1;

const WINDOWS_TICKS_PER_SECOND: i128 = 10_000_000;
const WINDOWS_UNIX_EPOCH_SECONDS: i128 = 11_644_473_600;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const INODE_MASK: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SrcId {
    volume: u64,
    file_id: [u8; 16],
}

struct WindowsFileInfo {
    src_id: SrcId,
    nlink: u32,
}

struct Inode {
    path: PathBuf,
    src_id: SrcId,
    nlookup: AtomicU64,
}

/// A filesystem backed by a Windows host directory.
pub struct HostFS {
    root: PathBuf,
    inodes: RwLock<HashMap<i64, Inode>>,
    src_to_ino: RwLock<HashMap<SrcId, i64>>,
}

pub struct HostFSFile {
    file: StdFile,
    ino: i64,
}

#[async_trait]
impl File for HostFSFile {
    async fn pread(&self, offset: u64, size: u64) -> Result<Vec<u8>> {
        let file = self.file.try_clone()?;
        tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; size as usize];
            let n = file.seek_read(&mut buf, offset)?;
            buf.truncate(n);
            Ok(buf)
        })
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
    }

    async fn pwrite(&self, offset: u64, data: &[u8]) -> Result<()> {
        let file = self.file.try_clone()?;
        let data = data.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut written = 0usize;
            while written < data.len() {
                let n = file.seek_write(&data[written..], offset + written as u64)?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "write returned zero",
                    ));
                }
                written += n;
            }
            Ok(())
        })
        .await
        .map_err(|e| Error::Internal(e.to_string()))??;
        Ok(())
    }

    async fn truncate(&self, size: u64) -> Result<()> {
        let file = self.file.try_clone()?;
        tokio::task::spawn_blocking(move || file.set_len(size))
            .await
            .map_err(|e| Error::Internal(e.to_string()))??;
        Ok(())
    }

    async fn fsync(&self) -> Result<()> {
        let file = self.file.try_clone()?;
        tokio::task::spawn_blocking(move || file.sync_all())
            .await
            .map_err(|e| Error::Internal(e.to_string()))??;
        Ok(())
    }

    async fn fstat(&self) -> Result<Stats> {
        let metadata = self.file.metadata()?;
        let info = query_file_info(&self.file)?;
        Ok(metadata_to_stats(&metadata, self.ino, info.nlink))
    }
}

impl HostFS {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.exists() {
            return Err(Error::BaseDirectoryNotFound(root.display().to_string()));
        }
        if !root.is_dir() {
            return Err(Error::NotADirectory(root.display().to_string()));
        }

        let root = root
            .canonicalize()
            .map_err(|e| Error::Internal(format!("failed to canonicalize root: {}", e)))?;
        let info = query_file_info_for_path(&root)?;

        let root_inode = Inode {
            path: root.clone(),
            src_id: info.src_id,
            nlookup: AtomicU64::new(1),
        };

        let mut inodes = HashMap::new();
        inodes.insert(ROOT_INO, root_inode);

        let mut src_to_ino = HashMap::new();
        src_to_ino.insert(info.src_id, ROOT_INO);

        Ok(Self {
            root,
            inodes: RwLock::new(inodes),
            src_to_ino: RwLock::new(src_to_ino),
        })
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    fn get_inode_path(&self, ino: i64) -> Result<PathBuf> {
        let inodes = self.inodes.read().unwrap();
        let inode = inodes.get(&ino).ok_or(FsError::NotFound)?;
        Ok(inode.path.clone())
    }

    fn get_or_create_inode(&self, path: PathBuf, src_id: SrcId) -> i64 {
        {
            let src_map = self.src_to_ino.read().unwrap();
            if let Some(&ino) = src_map.get(&src_id) {
                let inodes = self.inodes.read().unwrap();
                if let Some(inode) = inodes.get(&ino) {
                    inode.nlookup.fetch_add(1, Ordering::Relaxed);
                    return ino;
                }
            }
        }

        let mut candidate = fingerprint_inode(&src_id);
        let mut inodes = self.inodes.write().unwrap();
        while let Some(existing) = inodes.get(&candidate) {
            if existing.src_id == src_id {
                existing.nlookup.fetch_add(1, Ordering::Relaxed);
                return candidate;
            }
            candidate = next_probe_inode(candidate);
        }

        inodes.insert(
            candidate,
            Inode {
                path,
                src_id,
                nlookup: AtomicU64::new(1),
            },
        );
        self.src_to_ino.write().unwrap().insert(src_id, candidate);
        candidate
    }

    fn stats_for_path(&self, path: &Path) -> Result<Stats> {
        let metadata = fs::symlink_metadata(path)?;
        let info = query_file_info_for_path(path)?;
        let ino = if path == self.root {
            ROOT_INO
        } else {
            self.get_or_create_inode(path.to_path_buf(), info.src_id)
        };
        Ok(metadata_to_stats(&metadata, ino, info.nlink))
    }

    fn remove_inode(&self, ino: i64) {
        let mut inodes = self.inodes.write().unwrap();
        if let Some(inode) = inodes.remove(&ino) {
            self.src_to_ino.write().unwrap().remove(&inode.src_id);
        }
    }

    fn remove_cached_paths_under(&self, path: &Path) {
        let mut inodes = self.inodes.write().unwrap();
        let to_remove = inodes
            .iter()
            .filter_map(|(&ino, inode)| {
                if ino != ROOT_INO && (inode.path == path || inode.path.strip_prefix(path).is_ok())
                {
                    Some((ino, inode.src_id))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for (ino, _) in &to_remove {
            inodes.remove(ino);
        }
        drop(inodes);

        let mut src_map = self.src_to_ino.write().unwrap();
        for (_, src_id) in to_remove {
            src_map.remove(&src_id);
        }
    }

    fn update_cached_paths_after_rename(&self, old_path: &Path, new_path: &Path) {
        let mut inodes = self.inodes.write().unwrap();
        for inode in inodes.values_mut() {
            if inode.path == old_path {
                inode.path = new_path.to_path_buf();
            } else if let Ok(suffix) = inode.path.strip_prefix(old_path) {
                inode.path = new_path.join(suffix);
            }
        }
    }
}

#[async_trait]
impl FileSystem for HostFS {
    async fn lookup(&self, parent_ino: i64, name: &str) -> Result<Option<Stats>> {
        validate_component(name)?;
        let parent_path = self.get_inode_path(parent_ino)?;
        let child_path = parent_path.join(name);

        match self.stats_for_path(&child_path) {
            Ok(stats) => Ok(Some(stats)),
            Err(Error::Io(err)) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn getattr(&self, ino: i64) -> Result<Option<Stats>> {
        let path = match self.get_inode_path(ino) {
            Ok(path) => path,
            Err(_) => return Ok(None),
        };

        match self.stats_for_path(&path) {
            Ok(mut stats) => {
                stats.ino = ino;
                Ok(Some(stats))
            }
            Err(Error::Io(err)) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn readlink(&self, ino: i64) -> Result<Option<String>> {
        let path = match self.get_inode_path(ino) {
            Ok(path) => path,
            Err(_) => return Ok(None),
        };

        match fs::read_link(&path) {
            Ok(target) => Ok(Some(target.to_string_lossy().to_string())),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) if err.kind() == io::ErrorKind::InvalidInput => {
                Err(FsError::NotASymlink.into())
            }
            Err(err) => Err(err.into()),
        }
    }

    async fn readdir(&self, ino: i64) -> Result<Option<Vec<String>>> {
        let path = match self.get_inode_path(ino) {
            Ok(path) => path,
            Err(_) => return Ok(None),
        };

        let mut entries = match fs::read_dir(&path) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        entries.sort();
        Ok(Some(entries))
    }

    async fn readdir_plus(&self, ino: i64) -> Result<Option<Vec<DirEntry>>> {
        let path = match self.get_inode_path(ino) {
            Ok(path) => path,
            Err(_) => return Ok(None),
        };

        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        let mut result = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            match self.stats_for_path(&entry.path()) {
                Ok(stats) => result.push(DirEntry { name, stats }),
                Err(Error::Io(err)) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Some(result))
    }

    async fn chmod(&self, ino: i64, mode: u32) -> Result<()> {
        let path = self.get_inode_path(ino)?;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_readonly(mode & 0o222 == 0);
        fs::set_permissions(path, permissions)?;
        Ok(())
    }

    async fn chown(&self, _ino: i64, _uid: Option<u32>, _gid: Option<u32>) -> Result<()> {
        Err(Error::Internal(
            "chown is not supported by Windows HostFS".to_string(),
        ))
    }

    async fn utimens(&self, ino: i64, atime: TimeChange, mtime: TimeChange) -> Result<()> {
        let path = self.get_inode_path(ino)?;
        let file = open_attributes_handle(&path, FILE_WRITE_ATTRIBUTES)?;
        let atime = time_change_to_filetime(atime)?;
        let mtime = time_change_to_filetime(mtime)?;
        let atime_ptr = atime
            .as_ref()
            .map_or(std::ptr::null(), |value| value as *const FILETIME);
        let mtime_ptr = mtime
            .as_ref()
            .map_or(std::ptr::null(), |value| value as *const FILETIME);

        let ok = unsafe {
            SetFileTime(
                file.as_raw_handle() as HANDLE,
                std::ptr::null(),
                atime_ptr,
                mtime_ptr,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }

    async fn open(&self, ino: i64, flags: i32) -> Result<BoxedFile> {
        let path = self.get_inode_path(ino)?;
        let file = open_file_for_flags(&path, flags)?;
        Ok(Arc::new(HostFSFile { file, ino }))
    }

    async fn mkdir(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        _uid: u32,
        _gid: u32,
    ) -> Result<Stats> {
        validate_component(name)?;
        let parent_path = self.get_inode_path(parent_ino)?;
        let path = parent_path.join(name);
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(FsError::AlreadyExists.into());
            }
            Err(err) => return Err(err.into()),
        }

        let stats = self
            .lookup(parent_ino, name)
            .await?
            .ok_or(FsError::NotFound)?;
        if mode & 0o222 == 0 {
            self.chmod(stats.ino, mode).await?;
        }
        Ok(stats)
    }

    async fn create_file(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        _uid: u32,
        _gid: u32,
    ) -> Result<(Stats, BoxedFile)> {
        validate_component(name)?;
        let parent_path = self.get_inode_path(parent_ino)?;
        let path = parent_path.join(name);
        let file = open_create_truncate(&path)?;

        if mode & 0o222 == 0 {
            let mut permissions = file.metadata()?.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions)?;
        }

        let stats = self
            .lookup(parent_ino, name)
            .await?
            .ok_or(FsError::NotFound)?;
        let boxed: BoxedFile = Arc::new(HostFSFile {
            file,
            ino: stats.ino,
        });
        Ok((stats, boxed))
    }

    async fn mknod(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        _rdev: u64,
        uid: u32,
        gid: u32,
    ) -> Result<Stats> {
        let file_type = mode & super::S_IFMT;
        if file_type == 0 || file_type == S_IFREG {
            let (stats, _) = self.create_file(parent_ino, name, mode, uid, gid).await?;
            return Ok(stats);
        }

        Err(Error::Internal(
            "mknod for special files is not supported by Windows HostFS".to_string(),
        ))
    }

    async fn symlink(
        &self,
        parent_ino: i64,
        name: &str,
        target: &str,
        _uid: u32,
        _gid: u32,
    ) -> Result<Stats> {
        validate_component(name)?;
        let parent_path = self.get_inode_path(parent_ino)?;
        let link_path = parent_path.join(name);
        let target_path = Path::new(target);
        let target_probe = if target_path.is_absolute() {
            target_path.to_path_buf()
        } else {
            parent_path.join(target_path)
        };

        let result = if target_probe.is_dir() {
            std::os::windows::fs::symlink_dir(target, &link_path)
        } else {
            std::os::windows::fs::symlink_file(target, &link_path)
        };
        match result {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(FsError::AlreadyExists.into());
            }
            Err(err) => return Err(err.into()),
        }

        self.lookup(parent_ino, name)
            .await?
            .ok_or(FsError::NotFound.into())
    }

    async fn unlink(&self, parent_ino: i64, name: &str) -> Result<()> {
        validate_component(name)?;
        let parent_path = self.get_inode_path(parent_ino)?;
        let path = parent_path.join(name);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Err(FsError::NotFound.into()),
            Err(err) => Err(err.into()),
        }
    }

    async fn rmdir(&self, parent_ino: i64, name: &str) -> Result<()> {
        validate_component(name)?;
        let parent_path = self.get_inode_path(parent_ino)?;
        let path = parent_path.join(name);
        match fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Err(FsError::NotFound.into()),
            Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => {
                Err(FsError::NotEmpty.into())
            }
            Err(err) => Err(err.into()),
        }
    }

    async fn link(&self, ino: i64, newparent_ino: i64, newname: &str) -> Result<Stats> {
        validate_component(newname)?;
        let old_path = self.get_inode_path(ino)?;
        let newparent_path = self.get_inode_path(newparent_ino)?;
        let new_path = newparent_path.join(newname);

        match fs::hard_link(&old_path, &new_path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(FsError::AlreadyExists.into());
            }
            Err(err) => return Err(err.into()),
        }

        self.lookup(newparent_ino, newname)
            .await?
            .ok_or(FsError::NotFound.into())
    }

    async fn rename(
        &self,
        oldparent_ino: i64,
        oldname: &str,
        newparent_ino: i64,
        newname: &str,
    ) -> Result<()> {
        validate_component(oldname)?;
        validate_component(newname)?;
        let oldparent_path = self.get_inode_path(oldparent_ino)?;
        let newparent_path = self.get_inode_path(newparent_ino)?;
        let old_path = oldparent_path.join(oldname);
        let new_path = newparent_path.join(newname);

        if old_path == new_path {
            return Ok(());
        }

        let old_metadata = match fs::symlink_metadata(&old_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(FsError::NotFound.into());
            }
            Err(err) => return Err(err.into()),
        };
        let old_is_dir = old_metadata.is_dir() && !old_metadata.file_type().is_symlink();

        let new_metadata = match fs::symlink_metadata(&new_path) {
            Ok(metadata) => Some(metadata),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };

        if let Some(metadata) = &new_metadata {
            let new_is_dir = metadata.is_dir() && !metadata.file_type().is_symlink();
            if new_is_dir && !old_is_dir {
                return Err(FsError::IsADirectory.into());
            }
            if !new_is_dir && old_is_dir {
                return Err(FsError::NotADirectory.into());
            }
        }

        match fs::rename(&old_path, &new_path) {
            Ok(()) => {
                if new_metadata.is_some() {
                    self.remove_cached_paths_under(&new_path);
                }
                self.update_cached_paths_after_rename(&old_path, &new_path);
                Ok(())
            }
            Err(_) if new_metadata.is_some() => {
                if let Some(metadata) = &new_metadata {
                    if metadata.is_dir() && !metadata.file_type().is_symlink() {
                        fs::remove_dir(&new_path)?;
                    } else {
                        fs::remove_file(&new_path)?;
                    }
                    self.remove_cached_paths_under(&new_path);
                }

                match fs::rename(&old_path, &new_path) {
                    Ok(()) => {
                        self.update_cached_paths_after_rename(&old_path, &new_path);
                        Ok(())
                    }
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {
                        Err(FsError::NotFound.into())
                    }
                    Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => {
                        Err(FsError::NotEmpty.into())
                    }
                    Err(err) => Err(err.into()),
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Err(FsError::NotFound.into()),
            Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => {
                Err(FsError::NotEmpty.into())
            }
            Err(err) => Err(err.into()),
        }
    }

    async fn statfs(&self) -> Result<FilesystemStats> {
        let mut free_available = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        let wide = path_to_wide(&self.root);
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_available,
                &mut total,
                &mut total_free,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(FilesystemStats {
            inodes: self.inodes.read().unwrap().len() as u64,
            bytes_used: total.saturating_sub(total_free),
        })
    }

    async fn forget(&self, ino: i64, nlookup: u64) {
        if ino == ROOT_INO {
            return;
        }

        let should_remove = {
            let inodes = self.inodes.read().unwrap();
            if let Some(inode) = inodes.get(&ino) {
                let old = inode.nlookup.fetch_sub(nlookup, Ordering::Relaxed);
                old <= nlookup
            } else {
                false
            }
        };

        if should_remove {
            self.remove_inode(ino);
        }
    }
}

fn validate_component(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('\\') || name.contains('/') {
        return Err(FsError::InvalidPath.into());
    }
    Ok(())
}

fn open_attributes_handle(path: &Path, access: u32) -> io::Result<StdFile> {
    let mut opts = OpenOptions::new();
    opts.access_mode(access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    opts.open(path)
}

fn open_file_for_flags(path: &Path, flags: i32) -> Result<StdFile> {
    let mut opts = OpenOptions::new();
    match flags & 0b11 {
        OPEN_READONLY => {
            opts.read(true);
        }
        OPEN_WRITEONLY => {
            opts.write(true);
        }
        OPEN_READWRITE => {
            opts.read(true).write(true);
        }
        _ => return Err(FsError::InvalidPath.into()),
    }
    opts.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    Ok(opts.open(path)?)
}

fn open_create_truncate(path: &Path) -> io::Result<StdFile> {
    let mut opts = OpenOptions::new();
    opts.read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    opts.open(path)
}

fn query_file_info_for_path(path: &Path) -> Result<WindowsFileInfo> {
    let file = open_attributes_handle(path, FILE_READ_ATTRIBUTES)?;
    query_file_info(&file)
}

fn query_file_info(file: &StdFile) -> Result<WindowsFileInfo> {
    let handle = file.as_raw_handle() as HANDLE;
    let mut basic: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let basic_ok = unsafe { GetFileInformationByHandle(handle, &mut basic) };
    if basic_ok == 0 {
        return Err(io::Error::last_os_error().into());
    }

    let mut id_info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let id_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            &mut id_info as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };

    let src_id = if id_ok != 0 {
        SrcId {
            volume: id_info.VolumeSerialNumber,
            file_id: id_info.FileId.Identifier,
        }
    } else {
        let file_index = ((basic.nFileIndexHigh as u64) << 32) | basic.nFileIndexLow as u64;
        let mut file_id = [0u8; 16];
        file_id[..8].copy_from_slice(&file_index.to_le_bytes());
        SrcId {
            volume: basic.dwVolumeSerialNumber as u64,
            file_id,
        }
    };

    Ok(WindowsFileInfo {
        src_id,
        nlink: basic.nNumberOfLinks.max(1),
    })
}

fn metadata_to_stats(metadata: &Metadata, ino: i64, nlink: u32) -> Stats {
    let attrs = metadata.file_attributes();
    let is_symlink = metadata.file_type().is_symlink();
    let file_type = if is_symlink {
        S_IFLNK
    } else if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
        S_IFDIR
    } else {
        S_IFREG
    };

    let base_perms = if file_type == S_IFDIR { 0o777 } else { 0o666 };
    let perms = if attrs & FILE_ATTRIBUTE_READONLY != 0 {
        base_perms & !0o222
    } else {
        base_perms
    };

    let (atime, atime_nsec) = filetime_to_unix(metadata.last_access_time());
    let (mtime, mtime_nsec) = filetime_to_unix(metadata.last_write_time());
    let (ctime, ctime_nsec) = filetime_to_unix(metadata.creation_time());

    Stats {
        ino,
        mode: file_type | perms,
        nlink,
        uid: 0,
        gid: 0,
        size: metadata.file_size() as i64,
        atime,
        mtime,
        ctime,
        atime_nsec,
        mtime_nsec,
        ctime_nsec,
        rdev: 0,
    }
}

fn filetime_to_unix(filetime: u64) -> (i64, u32) {
    if filetime == 0 {
        return (0, 0);
    }
    let intervals = filetime as i128;
    let secs = intervals / WINDOWS_TICKS_PER_SECOND - WINDOWS_UNIX_EPOCH_SECONDS;
    let nsec = (intervals % WINDOWS_TICKS_PER_SECOND) * 100;
    (secs as i64, nsec as u32)
}

fn time_change_to_filetime(change: TimeChange) -> Result<Option<FILETIME>> {
    let (secs, nsec) = match change {
        TimeChange::Omit => return Ok(None),
        TimeChange::Now => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
            (now.as_secs() as i64, now.subsec_nanos())
        }
        TimeChange::Set(secs, nsec) => (secs, nsec),
    };

    let intervals = (secs as i128 + WINDOWS_UNIX_EPOCH_SECONDS) * WINDOWS_TICKS_PER_SECOND
        + (nsec as i128 / 100);
    if intervals < 0 {
        return Err(Error::Internal(
            "Windows FILETIME cannot represent the requested timestamp".to_string(),
        ));
    }
    Ok(Some(u64_to_filetime(intervals as u64)))
}

fn u64_to_filetime(value: u64) -> FILETIME {
    FILETIME {
        dwLowDateTime: value as u32,
        dwHighDateTime: (value >> 32) as u32,
    }
}

fn fingerprint_inode(src_id: &SrcId) -> i64 {
    let mut hash = FNV_OFFSET;
    for byte in src_id
        .volume
        .to_le_bytes()
        .iter()
        .chain(src_id.file_id.iter())
    {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    let mut ino = hash & INODE_MASK;
    if ino <= ROOT_INO as u64 {
        ino += 2;
    }
    ino as i64
}

fn next_probe_inode(current: i64) -> i64 {
    let mut next = ((current as u64).wrapping_add(1)) & INODE_MASK;
    if next <= ROOT_INO as u64 {
        next = 2;
    }
    next as i64
}

fn path_to_wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DEFAULT_DIR_MODE, DEFAULT_FILE_MODE};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_hostfs_windows_basic() -> Result<()> {
        let dir = tempdir()?;
        let fs = HostFS::new(dir.path())?;

        let (_, file) = fs
            .create_file(ROOT_INO, "test.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file.pwrite(0, b"hello world").await?;

        let stats = fs.lookup(ROOT_INO, "test.txt").await?.unwrap();
        assert!(stats.is_file());

        let file = fs.open(stats.ino, OPEN_READONLY).await?;
        let data = file.pread(0, 100).await?;
        assert_eq!(data, b"hello world");

        Ok(())
    }

    #[tokio::test]
    async fn test_hostfs_windows_mkdir_readdir_plus() -> Result<()> {
        let dir = tempdir()?;
        let fs = HostFS::new(dir.path())?;

        let subdir = fs.mkdir(ROOT_INO, "subdir", DEFAULT_DIR_MODE, 0, 0).await?;
        assert!(subdir.is_directory());

        let (_, file_a) = fs
            .create_file(subdir.ino, "a.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file_a.pwrite(0, b"a").await?;
        let (_, file_b) = fs
            .create_file(subdir.ino, "b.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file_b.pwrite(0, b"b").await?;

        let entries = fs.readdir(subdir.ino).await?.unwrap();
        assert_eq!(entries, vec!["a.txt", "b.txt"]);

        let entries = fs.readdir_plus(subdir.ino).await?.unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.stats.is_file()));

        Ok(())
    }

    #[tokio::test]
    async fn test_hostfs_windows_stable_identity() -> Result<()> {
        let dir = tempdir()?;
        let fs = HostFS::new(dir.path())?;

        let (created, _) = fs
            .create_file(ROOT_INO, "same.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        let first = fs.lookup(ROOT_INO, "same.txt").await?.unwrap();
        let second = fs.lookup(ROOT_INO, "same.txt").await?.unwrap();

        assert_eq!(created.ino, first.ino);
        assert_eq!(first.ino, second.ino);
        assert_ne!(first.ino, ROOT_INO);

        Ok(())
    }

    #[tokio::test]
    async fn test_hostfs_windows_create_file_existing_truncates() -> Result<()> {
        let dir = tempdir()?;
        let fs = HostFS::new(dir.path())?;

        let (before, file) = fs
            .create_file(ROOT_INO, "existing.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file.pwrite(0, b"old content").await?;

        let (after, file) = fs
            .create_file(ROOT_INO, "existing.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        assert_eq!(before.ino, after.ino);
        assert!(file.pread(0, 100).await?.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_hostfs_windows_mutations() -> Result<()> {
        let dir = tempdir()?;
        let fs = HostFS::new(dir.path())?;

        let subdir = fs.mkdir(ROOT_INO, "dir", DEFAULT_DIR_MODE, 0, 0).await?;
        let (created, file) = fs
            .create_file(subdir.ino, "src.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file.pwrite(0, b"content").await?;
        drop(file);

        fs.rename(subdir.ino, "src.txt", subdir.ino, "dst.txt")
            .await?;
        assert!(fs.lookup(subdir.ino, "src.txt").await?.is_none());
        let renamed = fs.lookup(subdir.ino, "dst.txt").await?.unwrap();
        assert_eq!(created.ino, renamed.ino);

        let linked = fs.link(renamed.ino, subdir.ino, "hard.txt").await?;
        assert_eq!(renamed.ino, linked.ino);

        fs.unlink(subdir.ino, "hard.txt").await?;
        fs.unlink(subdir.ino, "dst.txt").await?;
        fs.rmdir(ROOT_INO, "dir").await?;
        assert!(fs.lookup(ROOT_INO, "dir").await?.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_hostfs_windows_rename_overwrite_file() -> Result<()> {
        let dir = tempdir()?;
        let fs = HostFS::new(dir.path())?;

        let (src, file) = fs
            .create_file(ROOT_INO, "src.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file.pwrite(0, b"src").await?;
        let (dst, file) = fs
            .create_file(ROOT_INO, "dst.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file.pwrite(0, b"dst").await?;
        drop(file);

        fs.rename(ROOT_INO, "src.txt", ROOT_INO, "dst.txt").await?;

        assert!(fs.lookup(ROOT_INO, "src.txt").await?.is_none());
        let renamed = fs.lookup(ROOT_INO, "dst.txt").await?.unwrap();
        assert_eq!(renamed.ino, src.ino);
        assert!(fs.getattr(dst.ino).await?.is_none());

        let file = fs.open(renamed.ino, OPEN_READONLY).await?;
        assert_eq!(file.pread(0, 100).await?, b"src");

        Ok(())
    }

    #[tokio::test]
    async fn test_hostfs_windows_rename_type_mismatch_preserves_destination() -> Result<()> {
        let dir = tempdir()?;
        let fs = HostFS::new(dir.path())?;

        let (_, file) = fs
            .create_file(ROOT_INO, "file.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file.pwrite(0, b"data").await?;
        fs.mkdir(ROOT_INO, "dir", DEFAULT_DIR_MODE, 0, 0).await?;

        let result = fs.rename(ROOT_INO, "file.txt", ROOT_INO, "dir").await;
        assert!(result.is_err());
        assert!(fs.lookup(ROOT_INO, "file.txt").await?.unwrap().is_file());
        assert!(fs.lookup(ROOT_INO, "dir").await?.unwrap().is_directory());

        let result = fs.rename(ROOT_INO, "dir", ROOT_INO, "file.txt").await;
        assert!(result.is_err());
        assert!(fs.lookup(ROOT_INO, "file.txt").await?.unwrap().is_file());
        assert!(fs.lookup(ROOT_INO, "dir").await?.unwrap().is_directory());

        Ok(())
    }

    #[tokio::test]
    async fn test_hostfs_windows_rename_missing_source_preserves_destination() -> Result<()> {
        let dir = tempdir()?;
        let fs = HostFS::new(dir.path())?;

        let (_, file) = fs
            .create_file(ROOT_INO, "dst.txt", DEFAULT_FILE_MODE, 0, 0)
            .await?;
        file.pwrite(0, b"dst").await?;

        let result = fs
            .rename(ROOT_INO, "missing.txt", ROOT_INO, "dst.txt")
            .await;
        assert!(result.is_err());

        let dst = fs.lookup(ROOT_INO, "dst.txt").await?.unwrap();
        let file = fs.open(dst.ino, OPEN_READONLY).await?;
        assert_eq!(file.pread(0, 100).await?, b"dst");

        Ok(())
    }
}
