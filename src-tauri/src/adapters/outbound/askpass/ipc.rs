//! Local IPC between the askpass helper process and the running application (§3.3).
//!
//! OpenSSH will not take a passphrase from us directly. It execs whatever `SSH_ASKPASS`
//! names and reads that program's stdout, so a passphrase the user typed in our window has
//! to travel: app → socket → helper → pipe → `ssh`. This module owns the app's end.
//!
//! Three decisions shape it, and each one narrows how long a secret exists and who can
//! reach it:
//!
//! - **A unix socket in a private directory**, not a TCP port and not an environment
//!   variable. An environment variable is readable from `/proc` on Linux by anything
//!   running as the user, and a TCP port is reachable by every process on the machine. The
//!   directory is `0700` and the socket path is unguessable, so the filesystem does the
//!   access control.
//! - **Armed, never asking.** The channel holds an answer the application already obtained;
//!   it never originates a prompt. A helper that connects when nothing is armed is
//!   answered with nothing, and `ssh` treats that as a refused credential — which is the
//!   truth, because no user was asked.
//! - **One answer per arming.** OpenSSH retries a passphrase up to three times. Serving the
//!   same rejected secret twice cannot succeed and only widens the window in which it
//!   exists, so the secret is taken, not read.

use crate::domain::request::Secret;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The variable the helper reads to find us. Passed to the child explicitly rather than
/// discovered, so a helper some other process launched cannot be answered by ours.
pub const SOCKET_ENV: &str = "APEX_ASKPASS_SOCKET";

struct Shared {
    /// The single answer, if the application has one to give.
    armed: Mutex<Option<Secret>>,
    /// Prompt texts OpenSSH sent, in order. The application uses the latest to caption its
    /// own dialog; the tests use them to assert a prompt happened at all.
    prompts: Mutex<Vec<String>>,
    closing: AtomicBool,
}

pub struct AskpassChannel {
    shared: Arc<Shared>,
    dir: PathBuf,
    socket: PathBuf,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl AskpassChannel {
    /// Bind a private socket and start answering.
    pub fn bind() -> std::io::Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "apex-askpass-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir)?;
        // Owner only. Without this the socket is connectable by any local user, and the
        // thing on the other end of it is a passphrase.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;

        let socket = dir.join("askpass.sock");
        let listener = UnixListener::bind(&socket)?;

        let shared = Arc::new(Shared {
            armed: Mutex::new(None),
            prompts: Mutex::new(Vec::new()),
            closing: AtomicBool::new(false),
        });

        let accept = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if shared.closing.load(Ordering::SeqCst) {
                        break;
                    }
                    match stream {
                        Ok(s) => answer(&shared, s),
                        Err(_) => break,
                    }
                }
            })
        };

        Ok(Self {
            shared,
            dir,
            socket,
            accept: Some(accept),
        })
    }

    /// Where the helper should connect. Absolute, because the helper inherits an
    /// unpredictable working directory from `ssh`.
    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// Hand the channel the answer for the next prompt.
    ///
    /// Called after the user has typed it, immediately before the assisted attempt. Arming
    /// replaces any previous answer, which is also what zeroes it: the old `Secret` drops.
    pub fn arm(&self, secret: Secret) {
        *self.shared.armed.lock().expect("armed lock") = Some(secret);
    }

    /// Forget any armed answer. Called when an attempt ends, however it ended.
    pub fn disarm(&self) {
        *self.shared.armed.lock().expect("armed lock") = None;
    }

    pub fn is_armed(&self) -> bool {
        self.shared.armed.lock().expect("armed lock").is_some()
    }

    /// Prompts OpenSSH has sent through the helper, oldest first.
    pub fn prompts(&self) -> Vec<String> {
        self.shared.prompts.lock().expect("prompts lock").clone()
    }
}

fn answer(shared: &Shared, stream: UnixStream) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut prompt = String::new();
    if reader.read_line(&mut prompt).is_err() {
        return;
    }
    shared
        .prompts
        .lock()
        .expect("prompts lock")
        .push(prompt.trim_end().to_string());

    // Take, not read: one answer per arming.
    let Some(secret) = shared.armed.lock().expect("armed lock").take() else {
        // Nothing armed means no user was asked. Closing without writing tells `ssh` the
        // credential could not be obtained, which is exactly what happened.
        return;
    };

    let mut out = stream;
    let _ = out.write_all(secret.expose());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
    // `secret` drops here and zeroes.
}

impl Drop for AskpassChannel {
    fn drop(&mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        // `accept` blocks and has no timeout, so it is woken by a connection rather than
        // left to be killed at process exit — a thread holding a live socket is exactly
        // what this type exists to avoid outliving.
        let _ = UnixStream::connect(&self.socket);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Stands in for the helper binary: connect, send a prompt, read whatever comes back.
    fn helper_asks(socket: &Path, prompt: &str) -> Option<String> {
        let mut s = UnixStream::connect(socket).ok()?;
        s.write_all(prompt.as_bytes()).ok()?;
        s.write_all(b"\n").ok()?;
        s.flush().ok()?;
        let mut answer = String::new();
        s.read_to_string(&mut answer).ok()?;
        let answer = answer.trim_end().to_string();
        if answer.is_empty() {
            None
        } else {
            Some(answer)
        }
    }

    #[test]
    fn an_armed_channel_answers_the_helper() {
        let c = AskpassChannel::bind().expect("bind");
        c.arm(Secret::new("correct horse"));
        assert_eq!(
            helper_asks(
                c.socket_path(),
                "Enter passphrase for key '/home/dev/.ssh/id_ed25519':"
            ),
            Some("correct horse".to_string())
        );
    }

    /// The prompt text is OpenSSH's, and it names the key file. That is what lets a user
    /// with several keys tell which credential is being asked for, so it is passed through
    /// rather than reworded.
    #[test]
    fn the_prompt_openssh_sent_is_recorded_verbatim() {
        let c = AskpassChannel::bind().expect("bind");
        c.arm(Secret::new("x"));
        let prompt = "Enter passphrase for key '/home/dev/.ssh/id_ed25519':";
        helper_asks(c.socket_path(), prompt);
        assert_eq!(c.prompts(), vec![prompt.to_string()]);
    }

    /// The property that keeps the channel from becoming a passphrase oracle: a helper
    /// nobody armed for gets nothing, whoever launched it.
    #[test]
    fn an_unarmed_channel_answers_nothing() {
        let c = AskpassChannel::bind().expect("bind");
        assert_eq!(helper_asks(c.socket_path(), "Enter passphrase:"), None);
    }

    /// OpenSSH asks up to three times. Serving a rejected secret again cannot succeed, and
    /// every extra serving is another copy in another process.
    #[test]
    fn an_answer_is_served_once_and_then_gone() {
        let c = AskpassChannel::bind().expect("bind");
        c.arm(Secret::new("once"));
        assert_eq!(helper_asks(c.socket_path(), "p"), Some("once".into()));
        assert_eq!(
            helper_asks(c.socket_path(), "p"),
            None,
            "a second prompt must not be answered from the same arming"
        );
        assert!(!c.is_armed());
    }

    #[test]
    fn disarming_withdraws_an_unused_answer() {
        let c = AskpassChannel::bind().expect("bind");
        c.arm(Secret::new("withdrawn"));
        c.disarm();
        assert_eq!(helper_asks(c.socket_path(), "p"), None);
    }

    /// The socket lives in a directory only its owner can enter. Without this the access
    /// control is the unguessability of a path, which is not access control.
    #[test]
    fn the_socket_directory_is_reachable_only_by_its_owner() {
        let c = AskpassChannel::bind().expect("bind");
        let dir = c.socket_path().parent().expect("a parent directory");
        let mode = std::fs::metadata(dir)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "group and other must have no access: {mode:o}"
        );
    }

    #[test]
    fn dropping_the_channel_removes_the_socket() {
        let path = {
            let c = AskpassChannel::bind().expect("bind");
            c.socket_path().to_path_buf()
        };
        assert!(!path.exists(), "a socket must not outlive the channel");
        assert!(!path.parent().unwrap().exists());
    }
}
