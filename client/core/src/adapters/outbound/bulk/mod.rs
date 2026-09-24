//! Outbound adapter: fetch bytes beside the protocol channel (A-BULK, A-BULKSIZE, §3.6).
//!
//! A second `ssh` invocation on the **existing** control master. A-BULK measured the cost: seven
//! invocations over one master authenticate once, while three that bypass it authenticate three
//! more times. The master is what A-B1 bought, and this is what spends it.

use crate::application::ports::bulk_transfer::{BulkError, BulkTransfer};
use crate::domain::workspace::ByteRange;
use async_trait::async_trait;

pub struct SshBulkTransfer {
    user: String,
    host: String,
}

impl SshBulkTransfer {
    pub fn new(user: String, host: String) -> Self {
        Self { user, host }
    }

    /// The argument vector for one bulk fetch.
    ///
    /// Separated from the spawn so it can be asserted without a process: the two constraints
    /// below are invisible in a successful run and expensive in a failing one.
    pub fn argv(user: &str, host: &str, remote: &str, range: Option<ByteRange>) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();

        // Attach to the transport's master; never become one.
        //
        // `ControlMaster=auto` would make this invocation a master, and a master with
        // `ControlPersist` backgrounds itself while still holding the stdout pipe it inherited —
        // so reading its output waits forever for an EOF that cannot arrive. F002 learned this
        // the expensive way on the deploy path; the same invocation shape carries the same trap.
        v.push("-o".into());
        v.push("ControlMaster=no".into());
        v.push("-o".into());
        v.push("ControlPath=~/.ssh/apex-%C".into());
        // No tty and no askpass behind this, so a prompt would hang a transfer with nobody able
        // to answer it.
        v.push("-o".into());
        v.push("BatchMode=yes".into());
        v.push(format!("{user}@{host}"));

        // `dd` rather than `cat` for a ranged fetch: it seeks instead of streaming and discarding,
        // which is what makes reading the tail of a large file proportional to the tail.
        let script = match range {
            None => format!("cat {}", shell_quote(remote)),
            Some(r) => format!(
                "dd if={} bs=1 skip={} count={} status=none",
                shell_quote(remote),
                r.offset,
                r.length
            ),
        };
        v.push(script);
        v
    }
}

/// Single-quote a path for a remote shell.
///
/// The path is workspace-relative input that has already been through containment on both sides,
/// but it reaches a shell here, and a shell is a second interpreter with its own rules. Quoting
/// is what keeps a filename containing `;` from becoming a command.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[async_trait]
impl BulkTransfer for SshBulkTransfer {
    async fn fetch(&self, remote: &str, range: Option<ByteRange>) -> Result<Vec<u8>, BulkError> {
        let argv = Self::argv(&self.user, &self.host, remote, range);
        // Blocking process I/O, moved off the runtime worker. The same reasoning as the SQLite
        // adapter: a blocking call on an async worker stalls everything else scheduled there,
        // and the runtime cannot tell that it has been stalled.
        let out = tokio::task::spawn_blocking(move || {
            std::process::Command::new("ssh")
                .args(&argv)
                .stdin(std::process::Stdio::null())
                .output()
        })
        .await
        .map_err(|e| BulkError::Transfer(format!("bulk task: {e}")))?
        .map_err(|e| BulkError::Transfer(format!("spawning ssh: {e}")))?;

        if out.status.success() {
            return Ok(out.stdout);
        }
        // A failure is reported rather than returning a short read, because a short read presents
        // as a corrupt file whose digest simply will not match — a symptom that says nothing
        // about the cause.
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.stdout.is_empty() && stderr.contains("No such file") {
            Err(BulkError::NotFound)
        } else {
            Err(BulkError::Transfer(format!(
                "remote read exited {:?} after {} bytes: {}",
                out.status.code(),
                out.stdout.len(),
                stderr.trim()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bulk_fetch_attaches_to_the_master_and_never_becomes_one() {
        let argv = SshBulkTransfer::argv("dev", "build-01", "/w/big.bin", None);
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "-o" && w[1] == "ControlMaster=no"),
            "without this the invocation becomes a master, backgrounds itself holding the \
             inherited stdout pipe, and the read waits forever for an EOF that cannot arrive: \
             {argv:?}"
        );
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "-o" && w[1] == "ControlPath=~/.ssh/apex-%C"),
            "and it must reuse F001's master rather than authenticating again: {argv:?}"
        );
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "-o" && w[1] == "BatchMode=yes"),
            "a prompt would hang a transfer nobody can answer: {argv:?}"
        );
    }

    #[test]
    fn a_ranged_fetch_seeks_rather_than_streaming_the_whole_file() {
        let argv = SshBulkTransfer::argv(
            "dev",
            "h",
            "/w/big.bin",
            Some(ByteRange {
                offset: 1_000_000,
                length: 100,
            }),
        );
        let script = argv.last().expect("the remote script");
        assert!(script.contains("skip=1000000"), "{script}");
        assert!(script.contains("count=100"), "{script}");
        assert!(
            !script.starts_with("cat "),
            "streaming and discarding a megabyte to read a hundred bytes makes reading the tail \
             of a large file proportional to the whole"
        );
    }

    #[test]
    fn a_path_reaching_the_remote_shell_is_quoted() {
        let argv = SshBulkTransfer::argv("dev", "h", "/w/a;rm -rf ~/b", None);
        let script = argv.last().unwrap();
        assert!(
            script.contains(r"'/w/a;rm -rf ~/b'"),
            "containment stops a path escaping the workspace; quoting stops it escaping the \
             shell, which is a second interpreter with its own rules: {script}"
        );
        assert_eq!(
            shell_quote("it's"),
            r"'it'\''s'",
            "an embedded quote must not end the quoting"
        );
    }
}
