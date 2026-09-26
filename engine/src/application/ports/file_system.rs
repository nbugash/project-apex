//! Outbound port: the filesystem.
//!
//! **Synchronous.** The engine has no async runtime and does not need one: it reads one frame,
//! answers it, reads the next, and the expensive work is not supposed to be on this channel at
//! all (§4.6, A-BULKSIZE). `read_dir` and `read` on a local disk are CPU and page cache, not
//! network latency, so async would move them to a blocking pool and arrive back where it started
//! — one runtime heavier on a binary that is transferred on every first connect.

use std::io;
use std::path::{Path, PathBuf};

/// What a directory entry is, before it becomes a wire type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: u64,
    pub modified: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMeta {
    pub is_directory: bool,
    pub size: u64,
    pub modified: i64,
}

pub trait FileSystem: Send + Sync {
    /// Resolve symlinks and `.` components. Fails if the path does not exist, which is what the
    /// two-stage containment check in `domain::path` depends on.
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf>;
    fn read_dir(&self, p: &Path) -> io::Result<Vec<RawEntry>>;
    fn metadata(&self, p: &Path) -> io::Result<RawMeta>;
    /// Read at most `len` bytes from `offset`. A range past the end yields no bytes rather than
    /// an error, which is what lets a caller scroll toward the end without a size race.
    fn read_range(&self, p: &Path, offset: u64, len: u64) -> io::Result<Vec<u8>>;

    /// Every byte of the file.
    ///
    /// Separate from `read_range` rather than expressed as one: a write compares the whole
    /// file's hash, and assembling it from ranges would race a file being changed between them.
    fn read_all(&self, p: &Path) -> io::Result<Vec<u8>>;

    /// Replace the file's contents so that a failure leaves the previous contents intact.
    ///
    /// A capability, not a technique: the port says what must be true afterwards, and the
    /// adapter chooses how. What must be true is that no reader ever observes a partial write --
    /// truncating and writing in place fails that at every point after the truncate.
    fn write_atomic(&self, p: &Path, bytes: &[u8]) -> io::Result<()>;
}
