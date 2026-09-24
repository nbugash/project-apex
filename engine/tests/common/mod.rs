//! An in-memory filesystem, so use cases are testable without a real tree.
//!
//! Deliberately **not** a stub: it models the behaviours the use cases depend on — canonicalising
//! away `.` components, distinguishing a directory from a file, and failing on demand — because a
//! fake that cannot fail tests only the happy path.
//!
//! It does **not** model symlinks. Those are proven against a real tree in `path_containment.rs`,
//! since resolving one is exactly the filesystem behaviour under test and a fake asserting our own
//! beliefs about it would prove nothing.

#![allow(dead_code)] // each test binary uses a different subset

pub mod fake_clock;
pub mod fake_watcher;

use apex_engine::application::ports::file_system::{FileSystem, RawEntry, RawMeta};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Default)]
pub struct FakeFileSystem {
    /// Absolute path -> contents. A directory is an entry with `None`.
    nodes: Mutex<BTreeMap<String, Option<Vec<u8>>>>,
    /// When set, every call fails with this kind.
    fail_with: Mutex<Option<io::ErrorKind>>,
}

impl FakeFileSystem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dir(&self, path: &str) -> &Self {
        self.nodes.lock().unwrap().insert(path.to_string(), None);
        self
    }

    pub fn file(&self, path: &str, bytes: &[u8]) -> &Self {
        // Parent directories exist implicitly, as they do on a real filesystem.
        let mut here = String::new();
        let mut parts: Vec<&str> = path.split('/').collect();
        parts.pop();
        for p in parts {
            if p.is_empty() {
                continue;
            }
            here.push('/');
            here.push_str(p);
            self.nodes
                .lock()
                .unwrap()
                .entry(here.clone())
                .or_insert(None);
        }
        self.nodes
            .lock()
            .unwrap()
            .insert(path.to_string(), Some(bytes.to_vec()));
        self
    }

    /// Delete a node, so a root can vanish underneath a registration (FR-038).
    pub fn remove(&self, path: &str) {
        self.nodes.lock().unwrap().remove(path);
    }

    /// Make every subsequent call fail. For the paths that must survive an unusable disk.
    pub fn fail(&self, kind: io::ErrorKind) {
        *self.fail_with.lock().unwrap() = Some(kind);
    }

    fn guard(&self) -> io::Result<()> {
        match *self.fail_with.lock().unwrap() {
            Some(k) => Err(io::Error::new(k, "fake filesystem was told to fail")),
            None => Ok(()),
        }
    }

    fn norm(p: &Path) -> String {
        let mut out = Vec::new();
        for c in p.to_string_lossy().split('/') {
            match c {
                "" | "." => continue,
                ".." => {
                    out.pop();
                }
                x => out.push(x.to_string()),
            }
        }
        format!("/{}", out.join("/"))
    }
}

impl FileSystem for FakeFileSystem {
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        self.guard()?;
        let key = Self::norm(p);
        if self.nodes.lock().unwrap().contains_key(&key) {
            Ok(PathBuf::from(key))
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "no such path"))
        }
    }

    fn read_dir(&self, p: &Path) -> io::Result<Vec<RawEntry>> {
        self.guard()?;
        let base = Self::norm(p);
        let prefix = if base == "/" {
            "/".to_string()
        } else {
            format!("{base}/")
        };
        let nodes = self.nodes.lock().unwrap();
        let mut out = Vec::new();
        for (path, content) in nodes.iter() {
            let Some(rest) = path.strip_prefix(&prefix) else {
                continue;
            };
            if rest.is_empty() || rest.contains('/') {
                continue; // not an immediate child
            }
            out.push(RawEntry {
                name: rest.to_string(),
                is_directory: content.is_none(),
                size: content.as_ref().map(|c| c.len() as u64).unwrap_or(0),
                modified: 0,
            });
        }
        Ok(out)
    }

    fn metadata(&self, p: &Path) -> io::Result<RawMeta> {
        self.guard()?;
        let key = Self::norm(p);
        match self.nodes.lock().unwrap().get(&key) {
            Some(Some(c)) => Ok(RawMeta {
                is_directory: false,
                size: c.len() as u64,
                modified: 0,
            }),
            Some(None) => Ok(RawMeta {
                is_directory: true,
                size: 0,
                modified: 0,
            }),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "no such path")),
        }
    }

    fn read_range(&self, p: &Path, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        self.guard()?;
        let key = Self::norm(p);
        match self.nodes.lock().unwrap().get(&key) {
            Some(Some(c)) => {
                let start = (offset as usize).min(c.len());
                let end = start.saturating_add(len as usize).min(c.len());
                // A range past the end yields no bytes rather than an error, which is what lets a
                // caller scroll toward the end without racing the file's size.
                Ok(c[start..end].to_vec())
            }
            Some(None) => Err(io::Error::new(
                io::ErrorKind::InvalidInput, // IsADirectory is 1.83+; MSRV here is 1.75
                "is a directory",
            )),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "no such path")),
        }
    }
}
