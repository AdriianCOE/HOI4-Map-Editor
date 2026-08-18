use ahash::AHashMap;
use defy::{ContextualError, Contextualize};
use fs_err::{File, OpenOptions};
use thiserror::Error;
use unicase::UniCase;
use unicase::bytemuck::TransparentWrapper;
use zip::read::ZipArchive;
use zip::write::ZipWriter;

use crate::error::Error;

use std::fmt;
use std::io::{self, prelude::*};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

fn reject_err<T, E>(
    result: Result<T, E>,
    predicate: impl FnOnce(&E) -> bool,
) -> Result<Option<T>, E> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(err) if predicate(&err) => Ok(None),
        Err(err) => Err(err),
    }
}

#[derive(Debug, Error)]
pub enum FilesError {
    #[error("io error: {0}")]
    Io(#[from] ContextualError<io::Error>),
    #[error("zip error: {0}")]
    Zip(#[from] ContextualError<zip::result::ZipError>),
    #[error(transparent)]
    Location(#[from] IntoLocationError),
    #[error("file could not be found: {}", .0.display())]
    FilesMapFileNotFound(PathBuf),
    #[error("file already exists: {}", .0.display())]
    FilesMapFileAlreadyExists(PathBuf),
}

impl FilesError {
    /// Convert `Result<T, Self>` to `Result<Option<T>, Self>` when `Self` is `Io` and is of the specified [`io::ErrorKind`] (`Err(Self)` becomes `Ok(None)` in this case).
    pub fn reject_io_err<T>(
        result: Result<T, Self>,
        io_error_kind: io::ErrorKind,
    ) -> Result<Option<T>, Self> {
        reject_err(
            result,
            |err| matches!(err, Self::Io(ContextualError { error, .. }) if error.kind() == io_error_kind),
        )
    }
}

pub fn is_zip_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

pub fn is_core_file(path: &Path) -> bool {
    path.file_name().is_some_and(|file_name| {
        file_name.eq_ignore_ascii_case("provinces.bmp")
            || file_name.eq_ignore_ascii_case("definition.csv")
    })
}

#[derive(Debug, Clone)]
pub enum Location {
    ZipArchive(PathBuf),
    Directory(PathBuf),
}

impl Location {
    pub fn as_path(&self) -> &Path {
        match self {
            Location::ZipArchive(path) => path,
            Location::Directory(path) => path,
        }
    }

    pub fn into_pathbuf(self) -> PathBuf {
        match self {
            Location::ZipArchive(path) => path,
            Location::Directory(path) => path,
        }
    }

    fn from_path(path: impl Into<PathBuf>) -> Result<Self, IntoLocationError> {
        let mut path = path.into();
        let metadata = path.metadata().context("failed to read metadata")?;

        if metadata.is_file() {
            if is_zip_file(&path) {
                return Ok(Location::ZipArchive(path));
            };

            if is_core_file(&path) {
                path.pop();
                return Ok(Location::Directory(path));
            };
        };

        if metadata.is_dir() {
            return Ok(Location::Directory(path));
        };

        Err(IntoLocationError::Invalid(path))
    }

    pub fn manipulate_files<R>(
        self,
        operation: impl FnOnce(&mut FilesAbstraction) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let mut files = FilesAbstraction::new(self)?;
        let result = operation(&mut files)?;
        files.dispose()?;
        Ok(result)
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Location::ZipArchive(path) => write!(f, "zip archive {}", path.display()),
            Location::Directory(path) => write!(f, "directory {}", path.display()),
        }
    }
}

pub trait IntoLocation {
    fn into_location(self) -> Result<Location, FilesError>;
}

impl IntoLocation for Location {
    fn into_location(self) -> Result<Location, FilesError> {
        Ok(self)
    }
}

impl<T> IntoLocation for T
where
    T: Into<PathBuf>,
{
    fn into_location(self) -> Result<Location, FilesError> {
        Location::from_path(self).map_err(FilesError::from)
    }
}

#[derive(Debug, Error)]
pub enum IntoLocationError {
    #[error("cannot extract location: {0}")]
    Io(#[from] ContextualError<io::Error>),
    #[error("cannot extract location from path {}: not a valid location", .0.display())]
    Invalid(PathBuf),
}

pub type FileHandleDyn<'h> = Box<dyn FileHandle + 'h>;

pub trait FileHandle: Read + Write + Seek {}

impl<T> FileHandle for T where T: Read + Write + Seek {}

#[derive(Debug)]
pub enum FilesAbstraction {
    Directory {
        root: PathBuf,
    },
    ZipArchive {
        path: PathBuf,
        zip: ZipArchiveFilesMap,
        dirty: bool,
    },
}

impl FilesAbstraction {
    pub fn new(location: impl IntoLocation) -> Result<Self, FilesError>
    where
        Self: Sized,
    {
        match location.into_location()? {
            Location::Directory(root) => {
                let root = canonicalize(root)?;
                Ok(Self::Directory { root })
            }
            Location::ZipArchive(path) => {
                let mut zip = ZipArchiveFilesMap::from_fs(&path)?;
                zip.set_comment(format!("Generated by {}", crate::APPNAME));
                Ok(Self::ZipArchive {
                    path,
                    zip,
                    dirty: false,
                })
            }
        }
    }

    pub fn open_file(&mut self, name: impl AsRef<Path>) -> Result<FileHandleDyn<'_>, FilesError> {
        match self {
            Self::Directory { root } => open_file(root.join(name)).map(file_to_file_handle),
            Self::ZipArchive { zip, .. } => zip.get_or_err_mut(name).map(buf_to_file_handle),
        }
    }

    pub fn open_file_maybe_not_found(
        &mut self,
        name: impl AsRef<Path>,
    ) -> Result<Option<FileHandleDyn<'_>>, FilesError> {
        match self {
            Self::Directory { root } => {
                open_file_maybe_not_found(root.join(name)).map(|o| o.map(file_to_file_handle))
            }
            Self::ZipArchive { zip, .. } => Ok(zip.get_mut(name).map(buf_to_file_handle)),
        }
    }

    pub fn create_file(&mut self, name: impl AsRef<Path>) -> Result<FileHandleDyn<'_>, FilesError> {
        match self {
            Self::Directory { root } => create_file(root.join(name)).map(file_to_file_handle),
            Self::ZipArchive { zip, dirty, .. } => {
                *dirty = true;
                Ok(buf_to_file_handle(zip.get_or_insert_new(name.as_ref())))
            }
        }
    }

    pub fn create_file_new(
        &mut self,
        name: impl AsRef<Path>,
    ) -> Result<FileHandleDyn<'_>, FilesError> {
        match self {
            Self::Directory { root } => create_file_new(root.join(name)).map(file_to_file_handle),
            Self::ZipArchive { zip, dirty, .. } => {
                *dirty = true;
                zip.insert_or_err(name, Vec::new()).map(buf_to_file_handle)
            }
        }
    }

    pub fn create_file_new_maybe_already_exists(
        &mut self,
        name: impl AsRef<Path>,
    ) -> Result<Option<FileHandleDyn<'_>>, FilesError> {
        match self {
            Self::Directory { root } => create_file_new_maybe_already_exists(root.join(name))
                .map(|o| o.map(file_to_file_handle)),
            Self::ZipArchive { zip, dirty, .. } => {
                *dirty = true;
                Ok(zip.insert(name, Vec::new()).map(buf_to_file_handle))
            }
        }
    }

    pub fn dispose(self) -> Result<(), FilesError> {
        match self {
            Self::Directory { .. } => Ok(()),
            Self::ZipArchive {
                path,
                zip,
                dirty: true,
            } => zip.to_fs(path),
            Self::ZipArchive { dirty: false, .. } => Ok(()),
        }
    }
}

#[inline]
fn file_to_file_handle(file: File) -> FileHandleDyn<'static> {
    Box::new(file) as Box<dyn FileHandle>
}

#[inline]
fn buf_to_file_handle(buf: &mut Vec<u8>) -> FileHandleDyn<'_> {
    Box::new(io::Cursor::new(buf)) as Box<dyn FileHandle>
}

#[derive(Debug, Clone)]
pub struct FilesMap {
    map: AHashMap<UniCase<PathBuf>, Vec<u8>>,
}

impl FilesMap {
    pub fn new() -> Self {
        FilesMap {
            map: AHashMap::default(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        FilesMap {
            map: AHashMap::with_capacity(capacity),
        }
    }

    pub fn get(&self, name: impl AsRef<Path>) -> Option<&Vec<u8>> {
        self.map.get(UniCase::wrap_ref(name.as_ref()))
    }

    pub fn get_mut(&mut self, name: impl AsRef<Path>) -> Option<&mut Vec<u8>> {
        self.map.get_mut(UniCase::wrap_ref(name.as_ref()))
    }

    pub fn get_or_insert(&mut self, name: impl Into<PathBuf>) -> &mut Vec<u8> {
        self.map.entry(UniCase::wrap(name.into())).or_default()
    }

    pub fn get_or_insert_new(&mut self, name: impl Into<PathBuf>) -> &mut Vec<u8> {
        self.map
            .entry(UniCase::wrap(name.into()))
            .and_modify(Vec::clear)
            .or_default()
    }

    pub fn get_or_err(&self, name: impl AsRef<Path>) -> Result<&Vec<u8>, FilesError> {
        let name = name.as_ref();
        self.get(name)
            .ok_or_else(|| FilesError::FilesMapFileNotFound(name.to_owned()))
    }

    pub fn get_or_err_mut(&mut self, name: impl AsRef<Path>) -> Result<&mut Vec<u8>, FilesError> {
        let name = name.as_ref();
        self.get_mut(name)
            .ok_or_else(|| FilesError::FilesMapFileNotFound(name.to_owned()))
    }

    pub fn insert(&mut self, name: impl AsRef<Path>, buf: Vec<u8>) -> Option<&mut Vec<u8>> {
        use std::collections::hash_map::Entry;
        match self.map.entry(UniCase::wrap(name.as_ref().to_owned())) {
            Entry::Occupied(..) => None,
            Entry::Vacant(entry) => Some(entry.insert(buf)),
        }
    }

    pub fn insert_or_err(
        &mut self,
        name: impl AsRef<Path>,
        buf: Vec<u8>,
    ) -> Result<&mut Vec<u8>, FilesError> {
        let name = name.as_ref();
        self.insert(name, buf)
            .ok_or_else(|| FilesError::FilesMapFileAlreadyExists(name.to_owned()))
    }

    pub fn remove(&mut self, name: impl AsRef<Path>) -> Option<Vec<u8>> {
        self.map.remove(UniCase::wrap_ref(name.as_ref()))
    }

    pub fn remove_or_err(&mut self, name: impl AsRef<Path>) -> Result<Vec<u8>, FilesError> {
        let name = name.as_ref();
        self.remove(name)
            .ok_or_else(|| FilesError::FilesMapFileNotFound(name.to_owned()))
    }

    pub fn clear_all(&mut self) {
        self.map.clear();
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&Path, &Vec<u8>)> {
        self.map.iter().map(|(name, buf)| (name.as_ref(), buf))
    }

    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = (&Path, &mut Vec<u8>)> {
        self.map.iter_mut().map(|(name, buf)| (name.as_ref(), buf))
    }

    pub fn into_iter(self) -> impl ExactSizeIterator<Item = (PathBuf, Vec<u8>)> {
        self.map
            .into_iter()
            .map(|(name, buf)| (UniCase::peel(name), buf))
    }
}

#[derive(Debug, Clone)]
pub struct ZipArchiveFilesMap {
    comment: String,
    map: FilesMap,
}

impl ZipArchiveFilesMap {
    pub fn new() -> Self {
        ZipArchiveFilesMap {
            comment: String::new(),
            map: FilesMap::new(),
        }
    }

    pub fn with_comment(comment: impl Into<String>) -> Self {
        ZipArchiveFilesMap {
            comment: comment.into(),
            map: FilesMap::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        ZipArchiveFilesMap {
            comment: String::new(),
            map: FilesMap::with_capacity(capacity),
        }
    }

    pub fn with_capacity_and_comment(capacity: usize, comment: impl Into<String>) -> Self {
        ZipArchiveFilesMap {
            comment: comment.into(),
            map: FilesMap::with_capacity(capacity),
        }
    }

    pub fn get_comment(&self) -> &String {
        &self.comment
    }

    pub fn get_comment_mut(&mut self) -> &mut String {
        &mut self.comment
    }

    pub fn set_comment(&mut self, comment: impl AsRef<str>) {
        comment.as_ref().clone_into(&mut self.comment);
    }

    pub fn clear_comment(&mut self) {
        self.comment.clear()
    }

    pub fn from_fs(path: impl Into<PathBuf>) -> Result<Self, FilesError> {
        let path = path.into();
        match open_file_maybe_not_found(&path)? {
            Some(file) => ZipArchiveFilesMap::from_reader(&file),
            None => Ok(ZipArchiveFilesMap::new()),
        }
    }

    pub fn from_reader(reader: impl Read + Seek) -> Result<Self, FilesError> {
        let mut zip_reader = ZipArchive::new(reader).context("failed to open zip archive")?;
        let zip_file_comment = String::from_utf8_lossy(zip_reader.comment());
        let mut zip_archive_files_map =
            Self::with_capacity_and_comment(zip_reader.len(), zip_file_comment);

        for i in 0..zip_reader.len() {
            let mut zip_file = zip_reader
                .by_index(i)
                .context("failed to get zip archive file")?;
            if let Some(zip_file_name) = zip_file.enclosed_name().map(Path::to_owned) {
                let zip_file_buffer = zip_archive_files_map.get_or_insert_new(zip_file_name);
                io::copy(&mut zip_file, zip_file_buffer)
                    .context("failed to read zip archive file")?;
            };
        }

        Ok(zip_archive_files_map)
    }

    pub fn to_fs(&self, path: impl Into<PathBuf>) -> Result<(), FilesError> {
        self.to_writer(create_file(path)?)
    }

    pub fn to_writer(&self, writer: impl Write + Seek) -> Result<(), FilesError> {
        let mut zip_writer = ZipWriter::new(writer);
        zip_writer.set_comment(self.comment.as_str());

        for (zip_file_name, zip_file_buffer) in self.map.iter() {
            let zip_file_name = AsRef::<Path>::as_ref(zip_file_name).to_string_lossy();
            zip_writer
                .start_file(zip_file_name, Default::default())
                .context("failed to start zip file")?;
            zip_writer
                .write_all(zip_file_buffer)
                .context("failed to write zip file contents")?;
        }

        zip_writer.finish().context("failed to write zip archive")?;

        Ok(())
    }
}

impl Deref for ZipArchiveFilesMap {
    type Target = FilesMap;

    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl DerefMut for ZipArchiveFilesMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

pub fn open_file(path: impl Into<PathBuf>) -> Result<File, FilesError> {
    OpenOptions::new()
        .read(true)
        .open(path)
        .context("failed to open file")
        .map_err(FilesError::from)
}

pub fn open_file_maybe_not_found(path: impl Into<PathBuf>) -> Result<Option<File>, FilesError> {
    FilesError::reject_io_err(open_file(path), io::ErrorKind::NotFound)
}

pub fn create_file(path: impl Into<PathBuf>) -> Result<File, FilesError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .context("failed to create file")
        .map_err(FilesError::from)
}

pub fn create_file_new(path: impl Into<PathBuf>) -> Result<File, FilesError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .context("failed to create new file")
        .map_err(FilesError::from)
}

pub fn create_file_new_maybe_already_exists(
    path: impl Into<PathBuf>,
) -> Result<Option<File>, FilesError> {
    FilesError::reject_io_err(create_file_new(path), io::ErrorKind::AlreadyExists)
}

pub fn canonicalize(path: impl AsRef<Path>) -> Result<PathBuf, FilesError> {
    let path = fs_err::canonicalize(path).context("failed to canonicalize path")?;

    #[cfg(windows)]
    let path = dunce::simplified(&path).to_owned();

    Ok(path)
}

pub(crate) fn write_complete_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_file(path, bytes, false)
}

pub(crate) fn write_new_complete_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_file(path, bytes, true)?;
    // A synced file alone does not make its new directory entry durable after
    // a power loss on Unix filesystems. The caller uses this for staged and
    // metadata files whose name must survive the next transaction barrier.
    sync_parent_directory(path)
}

fn write_file(path: &Path, bytes: &[u8], create_new: bool) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .create_new(create_new)
        .truncate(!create_new)
        .open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}

/// Makes a directory entry change for `path` durable on Unix filesystems.
///
/// Windows keeps its existing replacement API behavior; opening a directory as
/// a regular file there is intentionally avoided. This is a durability barrier,
/// not a claim that an entire multi-file transaction is filesystem-atomic.
pub(crate) fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent directory: {}", path.display()),
        )
    })?;
    sync_directory(parent)
}

/// Creates a directory tree and persists each newly-created directory entry on
/// Unix before it is used for transaction metadata or backups.
pub(crate) fn create_dir_all_durable(path: &Path) -> io::Result<()> {
    let mut created = Vec::new();
    let mut current = path;
    while !current.exists() {
        created.push(current.to_owned());
        current = current.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("directory has no parent: {}", path.display()),
            )
        })?;
    }
    std::fs::create_dir_all(path)?;
    for directory in created.into_iter().rev() {
        sync_parent_directory(&directory)?;
        sync_directory(&directory)?;
    }
    Ok(())
}

/// Removes a file and persists the parent directory entry change on Unix.
pub(crate) fn remove_file_durable(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            sync_parent_directory(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn atomic_replace_file(
    replacement: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> io::Result<()> {
    atomic_replace_existing(replacement, destination, backup)?;
    sync_parent_directory(destination)
}

pub(crate) fn atomic_move_new_file(source: &Path, destination: &Path) -> io::Result<()> {
    atomic_move_new(source, destination)?;
    sync_parent_directory(destination)
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn atomic_replace_existing(
    replacement: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> io::Result<()> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut core::ffi::c_void,
            reserved: *mut core::ffi::c_void,
        ) -> i32;
    }
    let destination_wide = wide(destination);
    let replacement_wide = wide(replacement);
    let backup_wide = backup.map(wide);
    let backup_ptr = backup_wide
        .as_ref()
        .map_or(std::ptr::null(), |path| path.as_ptr());
    let result = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            replacement_wide.as_ptr(),
            backup_ptr,
            0x0000_0001,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn atomic_move_new(source: &Path, destination: &Path) -> io::Result<()> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let source_wide = wide(source);
    let destination_wide = wide(destination);
    let result =
        unsafe { MoveFileExW(source_wide.as_ptr(), destination_wide.as_ptr(), 0x0000_0008) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace_existing(
    replacement: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> io::Result<()> {
    if let Some(backup) = backup {
        std::fs::copy(destination, backup)?;
        std::fs::OpenOptions::new()
            .read(true)
            .open(backup)?
            .sync_all()?;
        sync_parent_directory(backup)?;
    }
    std::fs::rename(replacement, destination)
}

#[cfg(not(windows))]
fn atomic_move_new(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::{
        ZipArchiveFilesMap, atomic_move_new_file, atomic_replace_file, create_dir_all_durable,
        remove_file_durable, write_new_complete_file,
    };
    use std::fs;

    #[test]
    fn reading_zip_does_not_rewrite_or_truncate_it() {
        let path = std::env::temp_dir().join(format!(
            "hoi4-state-editor-zip-read-{}.zip",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        let mut archive = ZipArchiveFilesMap::new();
        archive
            .get_or_insert_new("definition.csv")
            .extend_from_slice(b"data");
        archive.to_fs(&path).unwrap();
        let before = fs::read(&path).unwrap();

        let loaded = ZipArchiveFilesMap::from_fs(&path).unwrap();
        let after = fs::read(&path).unwrap();

        assert_eq!(loaded.get("definition.csv").unwrap(), b"data");
        assert_eq!(before, after);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn durable_publication_keeps_same_directory_replacements_and_creations_recoverable() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-state-editor-durable-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = root.join("project").join("map");
        create_dir_all_durable(&directory).unwrap();

        let destination = directory.join("definition.csv");
        let replacement = directory.join("definition.csv.stage");
        let backup = directory.join("definition.csv.rollback");
        write_new_complete_file(&destination, b"before").unwrap();
        write_new_complete_file(&replacement, b"after").unwrap();
        atomic_replace_file(&replacement, &destination, Some(&backup)).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"after");
        assert_eq!(fs::read(&backup).unwrap(), b"before");

        let staged_new = directory.join("new-state.stage");
        let published_new = directory.join("new-state.txt");
        write_new_complete_file(&staged_new, b"created").unwrap();
        atomic_move_new_file(&staged_new, &published_new).unwrap();
        assert_eq!(fs::read(&published_new).unwrap(), b"created");
        assert!(remove_file_durable(&published_new).unwrap());
        assert!(!published_new.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
