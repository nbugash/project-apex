//! `workspace/writeFile` against a real temp tree.
//!
//! A real tree rather than the fake, because every guarantee this method makes is about the
//! filesystem itself: that a refusal wrote nothing, that a failure left the previous content
//! whole, that a rename preserved the mode. A fake can be made to agree with any of those
//! without the real one doing so.
//!
//! **Every assertion reads the file back.** A test that checks the reply passes for an engine
//! that answers `-32004` and writes anyway, which is precisely the defect the base check exists
//! to prevent.

use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::{self, InMemoryRoots, WriteRefusal};
use apex_protocol::wire::{WorkspaceId, WriteFileParams};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

struct Tree {
    root: PathBuf,
    roots: InMemoryRoots,
    fs: Arc<StdFileSystem>,
    _dir: tempfile::TempDir,
}

impl Tree {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp");
        let root = dir.path().join("w");
        fs::create_dir_all(&root).unwrap();
        let fsp = Arc::new(StdFileSystem);
        let roots = InMemoryRoots::new(fsp.clone());
        roots
            .register("w1", root.to_str().unwrap())
            .expect("register");
        Self {
            root,
            roots,
            fs: fsp,
            _dir: dir,
        }
    }

    fn put(&self, name: &str, bytes: &[u8]) {
        fs::write(self.root.join(name), bytes).unwrap();
    }

    fn bytes(&self, name: &str) -> Vec<u8> {
        fs::read(self.root.join(name)).unwrap()
    }

    fn hash_of(&self, name: &str) -> String {
        workspace::hash_bytes(&self.bytes(name))
    }

    fn write(
        &self,
        name: &str,
        content: &str,
        base: &str,
    ) -> Result<apex_protocol::wire::WriteFileResult, WriteRefusal> {
        workspace::write_file(
            &self.roots,
            self.fs.as_ref(),
            &WriteFileParams {
                workspace_id: WorkspaceId("w1".into()),
                relative_path: name.into(),
                content: content.into(),
                base_sha256: base.into(),
            },
        )
    }
}

#[test]
fn a_matching_base_writes_and_returns_the_hash_of_what_landed() {
    let t = Tree::new();
    t.put("a.rs", b"old");
    let base = t.hash_of("a.rs");

    let result = t.write("a.rs", "new", &base).expect("should write");

    assert_eq!(
        t.bytes("a.rs"),
        b"new",
        "the file on disk must be what was sent"
    );
    assert_eq!(
        result.sha256,
        t.hash_of("a.rs"),
        "the returned hash must describe what is on disk, not what was requested"
    );
}

#[test]
fn a_mismatched_base_is_refused_and_the_file_is_untouched() {
    // The whole point of the method. Asserted on the bytes, because an engine that replies with
    // a conflict and writes anyway passes every assertion made about the reply.
    let t = Tree::new();
    t.put("a.rs", b"theirs");
    let before = t.bytes("a.rs");

    let refusal = t.write("a.rs", "mine", "not-the-current-hash").unwrap_err();

    assert!(matches!(refusal, WriteRefusal::Conflict), "got {refusal:?}");
    assert_eq!(
        t.bytes("a.rs"),
        before,
        "a refused write must not touch the file"
    );
}

#[test]
fn a_path_escaping_the_root_is_refused() {
    // Principle VI: the engine checks independently of whatever the client checked, because a
    // client-side check protects against bugs and never against a stale or hostile client.
    let t = Tree::new();
    let refusal = t.write("../escape.rs", "x", "any").unwrap_err();
    assert!(
        matches!(refusal, WriteRefusal::Request(_)),
        "an escape must be refused before anything is written: {refusal:?}"
    );
    assert!(!t.root.parent().unwrap().join("escape.rs").exists());
}

#[test]
fn a_symlink_out_of_the_root_is_refused() {
    // The escape a lexical check cannot see: no `..` anywhere in the path.
    let t = Tree::new();
    let outside = t._dir.path().join("outside.rs");
    fs::write(&outside, b"theirs").unwrap();
    std::os::unix::fs::symlink(&outside, t.root.join("link.rs")).unwrap();

    let refusal = t.write("link.rs", "mine", "any").unwrap_err();

    assert!(
        matches!(refusal, WriteRefusal::Request(_)),
        "got {refusal:?}"
    );
    assert_eq!(
        fs::read(&outside).unwrap(),
        b"theirs",
        "the target must be untouched"
    );
}

#[test]
fn content_above_the_limit_is_refused_before_anything_is_written() {
    let t = Tree::new();
    t.put("a.rs", b"old");
    let base = t.hash_of("a.rs");
    let huge = "x".repeat(workspace::MAX_WRITE_BYTES + 1);

    let refusal = t.write("a.rs", &huge, &base).unwrap_err();

    assert!(
        matches!(refusal, WriteRefusal::TooLarge { .. }),
        "got {refusal:?}"
    );
    assert_eq!(t.bytes("a.rs"), b"old");
}

#[test]
fn writing_a_file_that_does_not_exist_is_not_a_conflict() {
    // A missing file and a stale base are different problems with different remedies, and
    // `-32004` means the base mismatch and only that.
    let t = Tree::new();
    let refusal = t.write("absent.rs", "x", "any").unwrap_err();
    assert!(
        !matches!(refusal, WriteRefusal::Conflict),
        "a missing file reported as a conflict tells the developer a colleague edited a file \
         that was never there: {refusal:?}"
    );
}

#[test]
fn the_files_mode_survives_the_write() {
    // A rename replaces the inode, so without care the new file gets the process default and a
    // saved script silently stops being executable.
    use std::os::unix::fs::PermissionsExt;
    let t = Tree::new();
    t.put("run.sh", b"#!/bin/sh\n");
    fs::set_permissions(t.root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    let base = t.hash_of("run.sh");

    t.write("run.sh", "#!/bin/sh\necho hi\n", &base)
        .expect("should write");

    let mode = fs::metadata(t.root.join("run.sh"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755, "the executable bit was lost");
}

#[test]
fn no_temporary_file_is_left_behind() {
    let t = Tree::new();
    t.put("a.rs", b"old");
    let base = t.hash_of("a.rs");
    t.write("a.rs", "new", &base).expect("should write");

    let stray: Vec<_> = fs::read_dir(&t.root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "a.rs")
        .collect();
    assert!(stray.is_empty(), "left behind: {stray:?}");
}

#[test]
fn an_unchanged_write_still_returns_the_current_hash() {
    // Saving a file nobody changed is ordinary, and must not be mistaken for a conflict.
    let t = Tree::new();
    t.put("a.rs", b"same");
    let base = t.hash_of("a.rs");
    let result = t.write("a.rs", "same", &base).expect("should write");
    assert_eq!(result.sha256, base);
}

/// A filesystem that changes what it is given, which is the only way to tell "the hash of what
/// was written" apart from "the hash of what was sent".
///
/// Not hypothetical: a mount that translates line endings, a FUSE layer, or a filter driver all
/// do this. If the engine returned the request's hash, the client would adopt a base describing
/// bytes that are not on disk, and its next save would be refused for a conflict nobody caused.
struct MangleOnWrite(StdFileSystem);

impl apex_engine::application::ports::file_system::FileSystem for MangleOnWrite {
    fn canonicalize(&self, p: &std::path::Path) -> std::io::Result<PathBuf> {
        self.0.canonicalize(p)
    }
    fn read_dir(
        &self,
        p: &std::path::Path,
    ) -> std::io::Result<Vec<apex_engine::application::ports::file_system::RawEntry>> {
        self.0.read_dir(p)
    }
    fn metadata(
        &self,
        p: &std::path::Path,
    ) -> std::io::Result<apex_engine::application::ports::file_system::RawMeta> {
        self.0.metadata(p)
    }
    fn read_range(&self, p: &std::path::Path, offset: u64, len: u64) -> std::io::Result<Vec<u8>> {
        self.0.read_range(p, offset, len)
    }
    fn read_all(&self, p: &std::path::Path) -> std::io::Result<Vec<u8>> {
        self.0.read_all(p)
    }
    fn write_atomic(&self, p: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut altered = bytes.to_vec();
        altered.push(b'\n');
        self.0.write_atomic(p, &altered)
    }
}

#[test]
fn the_returned_hash_describes_the_disk_and_not_the_request() {
    let t = Tree::new();
    t.put("a.rs", b"old");
    let base = t.hash_of("a.rs");
    let mangling = MangleOnWrite(StdFileSystem);

    let result = workspace::write_file(
        &t.roots,
        &mangling,
        &WriteFileParams {
            workspace_id: WorkspaceId("w1".into()),
            relative_path: "a.rs".into(),
            content: "new".into(),
            base_sha256: base,
        },
    )
    .expect("should write");

    assert_eq!(
        result.sha256,
        t.hash_of("a.rs"),
        "the hash must describe what is on disk"
    );
    assert_ne!(
        result.sha256,
        workspace::hash_bytes(b"new"),
        "returning the request's hash would hand the client a base for bytes that are not there"
    );
}
