//! `workspace/readFile` and `workspace/stat` against a real temp tree.
//!
//! A real tree rather than the fake, because these are the behaviours where the filesystem itself
//! is the contract: seeking past the end, binary bytes surviving, and a file changing underneath
//! a read.

use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::{self, InMemoryRoots, ReadRefusal};
use apex_protocol::wire::{EntryKind, MAX_INLINE_READ};
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

    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.root.join(name), bytes).unwrap();
    }

    fn read(
        &self,
        rel: &str,
        offset: Option<u64>,
        length: Option<u64>,
    ) -> Result<apex_protocol::wire::ReadFileResult, ReadRefusal> {
        let p =
            workspace::resolve_request(&self.roots, self.fs.as_ref(), "w1", rel).expect("resolves");
        workspace::read_file(self.fs.as_ref(), &p, offset, length).expect("no io error")
    }

    fn stat(&self, rel: &str) -> apex_protocol::wire::StatResult {
        let p =
            workspace::resolve_request(&self.roots, self.fs.as_ref(), "w1", rel).expect("resolves");
        workspace::stat(self.fs.as_ref(), &p).expect("stat")
    }
}

fn decode(b64: &str) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut table = [255u8; 256];
    for (i, c) in T.iter().enumerate() {
        table[*c as usize] = i as u8;
    }
    let bytes: Vec<u8> = b64.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::new();
    for chunk in bytes.chunks(4) {
        let mut n: u32 = 0;
        for (i, b) in chunk.iter().enumerate() {
            n |= (table[*b as usize] as u32) << (18 - 6 * i);
        }
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

#[test]
fn binary_content_survives_byte_for_byte() {
    let t = Tree::new();
    // Bytes that are not valid UTF-8 anywhere: what an encoding guess would destroy silently.
    let payload: Vec<u8> = (0u8..=255).collect();
    t.write("image.png", &payload);

    let r = t.read("/image.png", None, None).expect("inline");
    assert_eq!(r.encoding, "base64", "there is no utf8 path (FR-003)");
    assert_eq!(
        decode(&r.content),
        payload,
        "build artifacts, images and PDFs are legal content, and an encoding guess corrupts them \
         silently"
    );
}

#[test]
fn a_range_returns_only_that_range_with_the_whole_files_size() {
    let t = Tree::new();
    t.write("a.txt", b"0123456789");
    let r = t.read("/a.txt", Some(3), Some(4)).expect("ranged");
    assert_eq!(decode(&r.content), b"3456");
    assert_eq!(
        r.total_size, 10,
        "the whole file's size, so a caller knows more follows"
    );
}

#[test]
fn a_range_past_the_end_yields_no_bytes_rather_than_an_error() {
    let t = Tree::new();
    t.write("a.txt", b"12345");
    let r = t
        .read("/a.txt", Some(99), Some(10))
        .expect("must not error");
    assert!(decode(&r.content).is_empty());
    assert_eq!(
        r.total_size, 5,
        "which is what lets a caller scroll toward the end without racing the file's size"
    );
}

#[test]
fn the_digest_describes_the_whole_file_not_the_returned_range() {
    let t = Tree::new();
    t.write("a.txt", b"0123456789");
    let whole = t.read("/a.txt", None, None).expect("whole");
    let part = t.read("/a.txt", Some(2), Some(3)).expect("part");
    assert_eq!(
        whole.sha256, part.sha256,
        "a caller assembling several ranges compares this across them; if it described the range \
         it would differ every time and detect nothing (FR-021)"
    );
}

/// The stat-then-read race, FR-021. Without this assertion the internal-consistency claim is
/// unverifiable: an implementation that returned a cached digest beside fresh bytes would pass
/// every other test in this file.
#[test]
fn a_file_rewritten_between_stat_and_read_is_detectable_by_its_digest() {
    let t = Tree::new();
    t.write("a.txt", b"original");
    let before = t.stat("/a.txt").sha256.expect("a file has a digest");

    t.write("a.txt", b"rewritten underneath");

    let after = t.read("/a.txt", None, None).expect("read");
    assert_ne!(
        before, after.sha256,
        "the digest must move with the content, so a caller can tell the file changed between \
         the confirmation and the read and discard what it assembled"
    );
    assert_eq!(decode(&after.content), b"rewritten underneath");
}

#[test]
fn a_read_above_the_inline_limit_is_refused_rather_than_truncated() {
    let t = Tree::new();
    let big = vec![b'x'; (MAX_INLINE_READ + 1) as usize];
    t.write("big.bin", &big);

    match t.read("/big.bin", None, None) {
        Err(ReadRefusal::TooLarge { total_size }) => {
            assert_eq!(total_size, big.len() as u64);
        }
        Ok(_) => panic!(
            "a silent truncation is a corrupt file the caller cannot see; the refusal is what \
             routes the read to the bulk path (A-BULKSIZE)"
        ),
    }
}

#[test]
fn a_range_within_the_limit_of_a_large_file_is_still_served_inline() {
    let t = Tree::new();
    let big = vec![b'x'; (MAX_INLINE_READ * 2) as usize];
    t.write("big.bin", &big);

    // The first screen of a large file goes through the channel; only the whole file is bulk.
    // That is what reconciles FR-023 with FR-025 — they govern different requests.
    let head = t
        .read("/big.bin", Some(0), Some(1024))
        .expect("head must be inline");
    assert_eq!(decode(&head.content).len(), 1024);
    assert_eq!(head.total_size, big.len() as u64);
}

#[test]
fn stat_omits_the_digest_for_a_directory() {
    let t = Tree::new();
    fs::create_dir(t.root.join("sub")).unwrap();
    let s = t.stat("/sub");
    assert_eq!(s.kind, EntryKind::Directory);
    assert_eq!(
        s.sha256, None,
        "there is nothing to hash and no caller that needs it"
    );
}

#[test]
fn an_empty_file_has_the_known_empty_digest() {
    let t = Tree::new();
    t.write("empty", b"");
    assert_eq!(
        t.stat("/empty").sha256.unwrap(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "an empty file is a file, not an absence"
    );
}
