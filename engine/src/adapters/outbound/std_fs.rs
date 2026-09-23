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
}
