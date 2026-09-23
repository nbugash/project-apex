//! The protocol against a real `sshd`. **Opt-in.**
//!
//! Two requirements point in opposite directions here, and this file is how both are met
//! rather than one quietly chosen over the other:
//!
//! - Constitution Principle VII names "the protocol against a real `sshd`" as the
//!   integration standard. Everything else in this suite talks to a mock through a pipe,
//!   which proves the transport's logic and proves nothing about the §3.1 invocation —
//!   whether the real client accepts those options, in that order, on this platform.
//! - SC-010 forbids the suite from *requiring* a remote host or a network.
//!
//! So this runs only when asked, with `APEX_REAL_SSHD=1`, and skips with a stated reason
//! otherwise. It binds loopback, which is why it cannot be part of the default run: the
//! network-free assertion in `transport_exchange.rs` would fail, correctly.
//!
//! ```sh
//! APEX_REAL_SSHD=1 cargo test --test transport_real_sshd -- --nocapture
//! ```

// A test that waits for a daemon to come up, on the thread doing the waiting and with no
// runtime in sight. The crate-wide ban on `std::thread::sleep` guards the interaction path,
// which this is not on.
#![allow(clippy::disallowed_methods)]

use apex_shell::adapters::outbound::openssh::OpenSshSpawner;
use apex_shell::application::ports::spawner::{ProcessSpawner, SpawnSpec};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Why this run is being skipped, or `None` when it can proceed.
fn skip_reason() -> Option<String> {
    if std::env::var("APEX_REAL_SSHD").unwrap_or_default() != "1" {
        return Some(
            "APEX_REAL_SSHD is not set to 1. This test binds loopback, which the default \
             suite must not need (SC-010)."
                .into(),
        );
    }
    for tool in ["/usr/sbin/sshd", "/usr/bin/ssh-keygen"] {
        if !Path::new(tool).exists() {
            return Some(format!("{tool} is not installed"));
        }
    }
    if Command::new("ssh").arg("-V").output().is_err() {
        return Some("no ssh client on PATH".into());
    }
    None
}

struct Sshd {
    dir: PathBuf,
    port: u16,
    client_key: PathBuf,
    known_hosts: PathBuf,
    child: Child,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // The control master persists for an hour by design (§3.1), so it is closed rather
        // than left holding a connection to a daemon that is about to disappear.
        let _ = Command::new("ssh")
            .arg("-O")
            .arg("exit")
            .arg("-o")
            .arg(format!("ControlPath={}/control-%C", self.dir.display()))
            .arg("-p")
            .arg(self.port.to_string())
            .arg(format!("{}@127.0.0.1", whoami()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn keygen(path: &Path) {
    let status = Command::new("/usr/bin/ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(path)
        .status()
        .expect("ssh-keygen");
    assert!(status.success(), "ssh-keygen failed for {}", path.display());
}

/// A private `sshd` on loopback, accepting one key, serving one command.
fn start_sshd() -> Sshd {
    let dir = std::env::temp_dir().join(format!("apex-sshd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    // 0700 on the directory and 0600 on the key below are the real protection. sshd's
    // StrictModes also walks the *ancestors* of the authorized_keys path and refuses any
    // that are group- or world-writable — and the temp directory is world-writable by
    // design, which no permission this test can set will change. So StrictModes is off in
    // the config, and the permissions are set properly anyway.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("0700");

    let host_key = dir.join("host_ed25519");
    let client_key = dir.join("client_ed25519");
    keygen(&host_key);
    keygen(&client_key);

    let authorized = dir.join("authorized_keys");
    std::fs::copy(dir.join("client_ed25519.pub"), &authorized).expect("authorized_keys");
    std::fs::set_permissions(&authorized, std::fs::Permissions::from_mode(0o600)).expect("0600");

    // Port 0 would be ideal, but sshd does not report the port it chose. Asking the kernel
    // for a free one and closing it immediately leaves a small race, which is why the
    // readiness loop below retries rather than assuming the bind succeeded.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        l.local_addr().expect("addr").port()
    };

    let config = dir.join("sshd_config");
    std::fs::write(
        &config,
        format!(
            "Port {port}\n\
             ListenAddress 127.0.0.1\n\
             HostKey {host}\n\
             AuthorizedKeysFile {auth}\n\
             PidFile {pid}\n\
             PasswordAuthentication no\n\
             KbdInteractiveAuthentication no\n\
             UsePAM no\n\
             StrictModes no\n\
             PermitUserEnvironment no\n\
             LogLevel ERROR\n",
            host = host_key.display(),
            auth = authorized.display(),
            pid = dir.join("sshd.pid").display(),
        ),
    )
    .expect("sshd_config");

    let child = Command::new("/usr/sbin/sshd")
        .arg("-D") // foreground, so killing the child kills the daemon
        .arg("-f")
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sshd");

    let known_hosts = dir.join("known_hosts");
    std::fs::write(&known_hosts, "").expect("known_hosts");

    let sshd = Sshd {
        dir,
        port,
        client_key,
        known_hosts,
        child,
    };

    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return sshd;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("sshd did not accept connections on port {port}");
}

/// The production §3.1 invocation, plus only what a test daemon on loopback needs.
///
/// The extra options are appended by the test rather than added to `OpenSshSpawner`,
/// because a port and an identity file are exactly the kind of thing that, once in the
/// production spawner "for testing", ends up used in production. The invocation under test
/// is still the real one, unedited.
fn invocation_for(sshd: &Sshd, remote_command: &str) -> Vec<String> {
    let spawner = OpenSshSpawner::new(remote_command);
    let spec = SpawnSpec {
        host: "127.0.0.1".into(),
        user: whoami(),
        assisted: false,
    };
    let produced = spawner.invocation(&spec);

    // The test's options go **first**, because ssh takes the first value it is given for
    // any option. Appending them looked right and was not: the production
    // `ControlPath=~/.ssh/apex-%C` won, so running this test left a master socket in the
    // developer's own ~/.ssh, persisting for the hour `ControlPersist=1h` asks for.
    let mut args: Vec<String> = vec![
        "-p".into(),
        sshd.port.to_string(),
        "-i".into(),
        sshd.client_key.display().to_string(),
        "-o".into(),
        format!("UserKnownHostsFile={}", sshd.known_hosts.display()),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-o".into(),
        format!("ControlPath={}/control-%C", sshd.dir.display()),
    ];
    // Then the production invocation, unedited — including its own ControlPath, which is
    // now the later and therefore ignored one.
    args.extend(produced);
    args
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".into())
}

/// A frame as §4.1 defines it.
fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

#[test]
fn the_normative_invocation_carries_frames_over_a_real_sshd() {
    if let Some(reason) = skip_reason() {
        eprintln!("SKIPPED: {reason}");
        return;
    }

    let sshd = start_sshd();
    // The remote "engine" is the mock daemon, executed on the far side of a real ssh
    // connection. Everything between — authentication, the option set, the channel, the
    // pipe — is real.
    let args = invocation_for(&sshd, env!("CARGO_BIN_EXE_apex-mock-daemon"));
    eprintln!("ssh {}", args.join(" "));

    let mut child = Command::new("ssh")
        .args(&args)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ssh");

    let mut stdin = child.stdin.take().expect("stdin");
    stdin
        .write_all(&frame(
            r#"{"jsonrpc":"2.0","id":"real-1","method":"engine/echo","params":{}}"#,
        ))
        .expect("write");
    stdin.flush().expect("flush");

    // Drained on its own thread, and reported on failure. Without it a failure here reads
    // "got \"\"", which says nothing — ssh always explains itself on stderr.
    let stderr = {
        let mut err = child.stderr.take().expect("stderr");
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            text
        })
    };

    let mut stdout = child.stdout.take().expect("stdout");
    let mut buf = vec![0u8; 4096];
    let mut seen = Vec::new();
    // The reply is one small frame; read until it is complete or the child ends.
    while !String::from_utf8_lossy(&seen).contains("real-1") {
        match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => seen.extend_from_slice(&buf[..n]),
        }
    }
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let diagnosis = stderr.join().unwrap_or_default();

    let text = String::from_utf8_lossy(&seen).to_string();
    assert!(
        text.starts_with("Content-Length: "),
        "the reply must be framed as §4.1 defines it, got {text:?}\nssh said: {diagnosis}"
    );
    assert!(
        text.contains(r#""id":"real-1""#),
        "the reply must carry the id it answers: {text}"
    );
}

/// The §3.1 option set is accepted by the installed client. This is the assertion the mock
/// cannot make: a typo in an option name, or an option a platform's OpenSSH does not know,
/// is invisible to every test that never runs `ssh`.
#[test]
fn the_installed_ssh_accepts_every_option_the_invocation_sets() {
    if let Some(reason) = skip_reason() {
        eprintln!("SKIPPED: {reason}");
        return;
    }

    let spawner = OpenSshSpawner::default();
    let spec = SpawnSpec {
        host: "127.0.0.1".into(),
        user: whoami(),
        assisted: false,
    };
    for option in spawner
        .invocation(&spec)
        .windows(2)
        .filter(|w| w[0] == "-o")
        .map(|w| w[1].clone())
    {
        // `-G` makes ssh parse the configuration and print it without connecting, so this
        // checks the option set alone.
        let out = Command::new("ssh")
            .args(["-G", "-o", &option, "127.0.0.1"])
            .env("LC_ALL", "C")
            .output()
            .expect("ssh -G");
        assert!(
            out.status.success(),
            "the installed ssh rejected {option:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
