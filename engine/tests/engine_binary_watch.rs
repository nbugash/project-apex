//! The shipped binary, driven over stdio the way the client drives it.
//!
//! Everything else in this suite composes the engine's parts in-process. That proves the parts
//! and says nothing about the binary the client actually launches -- and the two differ, because
//! `main.rs` is where composition happens and composition is where this project has repeatedly
//! found its defects. F011 needed this: the in-process watch test passed while the running
//! engine delivered no file events at all.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

struct Engine {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Engine {
    fn start(socket_dir: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ide-engine"))
            // Its own socket directory, so this test never joins an engine another test or a
            // developer's session left running -- which would make it a test of that process.
            .env("XDG_RUNTIME_DIR", socket_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the engine binary must be built");
        let stdin = child.stdin.take().expect("stdin");
        let out = BufReader::new(child.stdout.take().expect("stdout"));
        Self { child, stdin, out }
    }

    fn send(&mut self, body: &str) {
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).expect("write");
        self.stdin.flush().expect("flush");
    }

    /// One frame, or `None` once `limit` has passed with nothing readable.
    fn frame(&mut self, limit: Duration) -> Option<serde_json::Value> {
        let deadline = Instant::now() + limit;
        let mut length = 0usize;
        loop {
            if Instant::now() > deadline {
                return None;
            }
            let mut line = String::new();
            if self.out.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let trimmed = line.trim_end();
            if let Some(n) = trimmed.strip_prefix("Content-Length: ") {
                length = n.parse().ok()?;
            } else if trimmed.is_empty() && length > 0 {
                let mut body = vec![0u8; length];
                self.out.read_exact(&mut body).ok()?;
                return serde_json::from_slice(&body).ok();
            }
        }
    }

    /// Collect frames until one satisfies `want`, or the budget runs out.
    fn until(
        &mut self,
        budget: Duration,
        mut want: impl FnMut(&serde_json::Value) -> bool,
    ) -> Option<serde_json::Value> {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            let frame = self.frame(left)?;
            if want(&frame) {
                return Some(frame);
            }
        }
        None
    }
}

#[test]
fn the_running_engine_reports_a_created_file() {
    // The exact sequence the client performs: register, ask for git status, watch the root,
    // and then a file appears on the host.
    let sockets = tempfile::tempdir().expect("sockets");
    let repo = tempfile::tempdir().expect("repo");
    std::fs::write(repo.path().join("existing.rs"), "x\n").unwrap();
    let root = repo.path().to_string_lossy().into_owned();

    let mut engine = Engine::start(sockets.path());
    engine.send(&format!(
        r#"{{"jsonrpc":"2.0","id":"1","method":"workspace/register","params":{{"workspace_id":"w1","path":"{root}"}}}}"#
    ));
    assert!(
        engine
            .until(Duration::from_secs(10), |f| f["id"] == "1")
            .is_some(),
        "the engine never answered workspace/register"
    );

    engine.send(
        r#"{"jsonrpc":"2.0","id":"2","method":"workspace/watch","params":{"workspace_id":"w1","paths":["/"]}}"#,
    );
    let watched = engine
        .until(Duration::from_secs(10), |f| f["id"] == "2")
        .expect("the engine never answered workspace/watch");
    assert!(
        watched["result"]["watching"].as_u64().unwrap_or(0) >= 1,
        "the engine reported watching nothing: {watched}"
    );

    std::fs::write(repo.path().join("brand-new.rs"), "fresh\n").unwrap();

    let event = engine.until(Duration::from_secs(10), |f| {
        f["method"] == "workspace/onFileEvent"
    });
    let event = event.expect("the running engine sent no workspace/onFileEvent");
    let paths: Vec<String> = event["params"]["events"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| e["relative_path"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        paths.iter().any(|p| p == "/brand-new.rs"),
        "the event named {paths:?} rather than the created file"
    );
}
