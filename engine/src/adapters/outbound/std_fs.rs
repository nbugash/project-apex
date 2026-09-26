//! `std::fs`, synchronously.

use crate::application::ports::file_system::{FileSystem, RawEntry, RawMeta};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct StdFileSystem;

fn secs(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl FileSystem for StdFileSystem {
    fn canonicalize(&self, p: &Path) -> std::io::Result<PathBuf> {
        std::fs::canonicalize(p)
    }

    fn read_dir(&self, p: &Path) -> std::io::Result<Vec<RawEntry>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(p)? {
            let entry = entry?;
            // `metadata()` follows symlinks, which is what the tree should show: a symlinked
            // directory inside the workspace is a directory. Containment is enforced when the
            // path is *resolved*, not when it is listed.
            let Ok(m) = entry.metadata() else { continue };
            let Ok(name) = entry.file_name().into_string() else {
                continue; // not UTF-8; it cannot travel on the wire, so it is not listed
            };
            out.push(RawEntry {
                name,
                is_directory: m.is_dir(),
                size: m.len(),
                modified: secs(&m),
            });
        }
        Ok(out)
    }

    fn metadata(&self, p: &Path) -> std::io::Result<RawMeta> {
        let m = std::fs::metadata(p)?;
        Ok(RawMeta {
            is_directory: m.is_dir(),
            size: m.len(),
            modified: secs(&m),
        })
    }

    fn read_range(&self, p: &Path, offset: u64, len: u64) -> std::io::Result<Vec<u8>> {
        let mut f = std::fs::File::open(p)?;
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = Vec::new();
        f.take(len).read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn read_all(&self, p: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(p)
    }

    /// Write beside the target, then rename over it.
    ///
    /// **The temporary file shares the target's directory** on purpose. `rename` is atomic only
    /// within a filesystem; a temp file in `/tmp` would cross a mount on any machine where the
    /// workspace is not on the root filesystem, and the rename would fail rather than replace.
    ///
    /// The previous file's permissions are carried over, because the rename replaces the inode
    /// and a new file would otherwise take the process default -- saving a shell script would
    /// silently stop it being executable.
    ///
    /// Written with `std::fs` rather than the `tempfile` crate, which is a dev-dependency here.
    /// Promoting it would add a production dependency to a binary A-BOOT transfers on every
    /// first connect, to save about ten lines.
    fn write_atomic(&self, p: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir = p.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "no parent directory")
        })?;
        let name = p
            .file_name()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;

        // Unique per process and per attempt. Two engines writing the same file at the same
        // instant is not a case this protects against -- the base-hash check is what makes that
        // safe -- but two writes colliding on a temporary name would corrupt both.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut temp = dir.to_path_buf();
        temp.push(format!(
            ".{}.apex-{}-{}",
            name.to_string_lossy(),
            std::process::id(),
            stamp
        ));

        let mode = std::fs::metadata(p).ok().map(|m| m.permissions().mode());

        let write = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(bytes)?;
            // Synced before the rename: a rename that lands before the data does leaves a file
            // that is present, correctly named and empty after a crash.
            file.sync_all()?;
            drop(file);
            if let Some(mode) = mode {
                std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))?;
            }
            std::fs::rename(&temp, p)
        })();

        if write.is_err() {
            // Leaving the temporary behind would litter the developer's workspace with files
            // their tools would then index, lint and show in the tree.
            let _ = std::fs::remove_file(&temp);
        }
        write
    }
}
