//! Outbound adapter: running a process, with a pseudo-terminal or with pipes.
//!
//! **The only file in the repository permitted to name the mechanism.**
//! `engine/tests/pty_confinement.rs` fails the build otherwise, on the terms
//! `inotify_confinement.rs` sets for the watcher. Everything that decides anything -- how output
//! is chunked, when a producer is slowed, what the retention bound admits, when an escalation is
//! due -- sits above this line and is tested against `FakeRunner` with no process at all.
//!
//! # Between `fork` and `exec`
//!
//! Only async-signal-safe calls are permitted there, and **allocation is not one of them**. A
//! `malloc` in the child of a multi-threaded `fork` can deadlock on a lock another thread held
//! at the moment of the fork, and the engine has threads: the watcher, the escalation thread and
//! one reader per task. So every `CString` this needs is built *before* forking, and the child
//! path touches nothing but `setsid`, `dup2`, `ioctl`, `setrlimit`, `execvp` and `_exit`.

use std::ffi::CString;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::pty::{openpty, Winsize};
use nix::sys::resource::{setrlimit, Resource};
use nix::sys::signal::{killpg, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{
    close, dup2_stderr, dup2_stdin, dup2_stdout, execvpe, fork, pipe, setsid, ForkResult,
    Pid as NixPid,
};

use apex_protocol::wire::Pid;

use crate::application::ports::clock::Millis;
use crate::application::ports::task_runner::{
    ControlError, Exit, ReadOutcome, ResourceLimits, SpawnFailure, SpawnRequest, SpawnedTask,
    TaskControl, TaskOutput, TaskRunner,
};
use crate::domain::task::{Shape, Stream, TaskSignal};

/// Everything the child needs, allocated before the fork.
struct ChildPlan {
    program: CString,
    argv: Vec<CString>,
    envp: Vec<CString>,
    cwd: CString,
    limits: ResourceLimits,
}

fn to_c(s: &str) -> Result<CString, SpawnFailure> {
    // An interior nul cannot be passed to `exec`, and this is the last point at which it can be
    // reported as a refusal rather than becoming a truncated command line.
    CString::new(s).map_err(|_| SpawnFailure::NotExecutable)
}

fn errno_to_spawn(e: Errno) -> SpawnFailure {
    match e {
        Errno::ENOENT | Errno::EACCES | Errno::ENOEXEC => SpawnFailure::NotExecutable,
        Errno::ENOTDIR => SpawnFailure::CwdUnusable,
        Errno::ENFILE | Errno::EMFILE | Errno::ENODEV | Errno::ENXIO => SpawnFailure::NoDevice,
        other => SpawnFailure::Failed(std::io::Error::from(other).kind()),
    }
}

fn errno_to_control(e: Errno) -> ControlError {
    match e {
        // The process is gone. Not an error the caller surfaces: FR-019 makes stopping a stopped
        // task a success, and a notification has no response to carry a failure in.
        Errno::ESRCH | Errno::EPIPE | Errno::EBADF => ControlError::Gone,
        other => ControlError::Failed(std::io::Error::from(other).kind()),
    }
}

pub struct PtyRunner;

impl PtyRunner {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PtyRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRunner for PtyRunner {
    fn spawn(&self, request: &SpawnRequest<'_>) -> Result<SpawnedTask, SpawnFailure> {
        let Some(program) = request.command.first() else {
            return Err(SpawnFailure::NotExecutable);
        };

        // Allocated here, used after the fork. See the module comment.
        let plan = ChildPlan {
            program: to_c(program)?,
            argv: request
                .command
                .iter()
                .map(|a| to_c(a))
                .collect::<Result<Vec<_>, _>>()?,
            envp: request
                .env
                .iter()
                .map(|(k, v)| to_c(&format!("{k}={v}")))
                .collect::<Result<Vec<_>, _>>()?,
            cwd: to_c(&request.cwd.as_path().display().to_string())?,
            limits: request.limits,
        };

        match request.shape {
            Shape::Pty { cols, rows } => spawn_with_terminal(&plan, cols, rows),
            Shape::Pipes => spawn_with_pipes(&plan),
        }
    }
}

/// Apply the per-process ceilings, then replace the image. Never returns on success.
///
/// # Safety
/// Called only in the child of a `fork`, and calls only async-signal-safe functions.
unsafe fn become_the_child(plan: &ChildPlan) -> ! {
    // Its own process group, so a signal reaches the task and everything it spawns. This is how
    // FR-018 is met without a cgroup, and it must happen before `exec` or the child inherits the
    // engine's group and a stop would signal the engine.
    let _ = setsid();

    // Both the soft and the hard limit. A child may raise its own soft limit up to its hard one,
    // so a soft-only ceiling is one the bounded process can simply remove.
    let addr = plan.limits.address_space_bytes;
    let _ = setrlimit(Resource::RLIMIT_AS, addr, addr);
    // Required by FR-005a rather than chosen: a core dump is a crash report carrying the whole
    // environment, and the environment commonly carries credentials.
    let _ = setrlimit(
        Resource::RLIMIT_CORE,
        plan.limits.core_bytes,
        plan.limits.core_bytes,
    );

    if nix::libc::chdir(plan.cwd.as_ptr()) != 0 {
        nix::libc::_exit(127);
    }

    // `execvpe` and not `execvp`. The latter takes no environment and the child would inherit
    // this process's, silently discarding the set the use case merged -- which is FR-001's
    // environment and SC-025's redaction both quietly not happening.
    let _ = execvpe(&plan.program, &plan.argv, &plan.envp);
    // `execvp` only returns on failure, and the parent learns which failure from the exit code.
    nix::libc::_exit(127);
}

fn spawn_with_terminal(
    plan: &ChildPlan,
    cols: u16,
    rows: u16,
) -> Result<SpawnedTask, SpawnFailure> {
    let size = Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pair = openpty(Some(&size), None).map_err(errno_to_spawn)?;
    let (master, slave) = (pair.master, pair.slave);

    // SAFETY: the child path below calls only async-signal-safe functions and never allocates.
    match unsafe { fork() }.map_err(errno_to_spawn)? {
        ForkResult::Child => {
            // One device: the terminal is stdin, stdout and stderr at once, which is why a
            // `pty: true` task's streams arrive merged (A-TASKSTREAM).
            let _ = dup2_stdin(&slave);
            let _ = dup2_stdout(&slave);
            let _ = dup2_stderr(&slave);
            unsafe {
                nix::libc::ioctl(0, nix::libc::TIOCSCTTY as nix::libc::c_ulong, 0);
                become_the_child(plan)
            }
        }
        ForkResult::Parent { child } => {
            drop(slave);
            let shared = Arc::new(Shared::new(child, Some(master.as_raw_fd())));
            Ok(SpawnedTask {
                pid: Pid(child.as_raw()),
                output: Box::new(PtyOutput {
                    stdout: Some(master),
                    stderr: None,
                    always: Stream::Stdout,
                }),
                control: Arc::new(PtyControl { shared }),
            })
        }
    }
}

fn spawn_with_pipes(plan: &ChildPlan) -> Result<SpawnedTask, SpawnFailure> {
    let (stdin_r, stdin_w) = pipe().map_err(errno_to_spawn)?;
    let (stdout_r, stdout_w) = pipe().map_err(errno_to_spawn)?;
    let (stderr_r, stderr_w) = pipe().map_err(errno_to_spawn)?;

    // SAFETY: as above.
    match unsafe { fork() }.map_err(errno_to_spawn)? {
        ForkResult::Child => {
            let _ = dup2_stdin(&stdin_r);
            let _ = dup2_stdout(&stdout_w);
            let _ = dup2_stderr(&stderr_w);
            unsafe { become_the_child(plan) }
        }
        ForkResult::Parent { child } => {
            drop(stdin_r);
            drop(stdout_w);
            drop(stderr_w);
            let write_end = stdin_w.as_raw_fd();
            let shared = Arc::new(Shared::new(child, Some(write_end)));
            // Kept alive by the control half, which owns the writing end.
            std::mem::forget(stdin_w);
            Ok(SpawnedTask {
                pid: Pid(child.as_raw()),
                output: Box::new(PtyOutput {
                    stdout: Some(stdout_r),
                    stderr: Some(stderr_r),
                    always: Stream::Stdout,
                }),
                control: Arc::new(PtyControl { shared }),
            })
        }
    }
}

/// What the two halves share: the child's identity and the descriptor input goes to.
struct Shared {
    child: NixPid,
    /// The pty master, or the writing end of the stdin pipe.
    input: Mutex<Option<std::os::fd::RawFd>>,
    /// Whether this is a terminal, which decides whether a resize means anything.
    exit: Mutex<Option<Exit>>,
}

impl Shared {
    fn new(child: NixPid, input: Option<std::os::fd::RawFd>) -> Self {
        Self {
            child,
            input: Mutex::new(input),
            exit: Mutex::new(None),
        }
    }
}

/// The reading half. Owned by one thread and never shared.
struct PtyOutput {
    stdout: Option<OwnedFd>,
    stderr: Option<OwnedFd>,
    /// With a terminal there is one device, so everything is tagged `Stdout`.
    always: Stream,
}

impl TaskOutput for PtyOutput {
    fn read(&mut self, timeout: Millis, out: &mut Vec<u8>) -> ReadOutcome {
        let mut fds: Vec<(Stream, std::os::fd::RawFd)> = Vec::new();
        if let Some(fd) = &self.stdout {
            fds.push((self.always, fd.as_raw_fd()));
        }
        if let Some(fd) = &self.stderr {
            fds.push((Stream::Stderr, fd.as_raw_fd()));
        }
        if fds.is_empty() {
            return ReadOutcome::Ended;
        }

        let mut poll_fds: Vec<PollFd> = fds
            .iter()
            .map(|(_, fd)| {
                // SAFETY: the descriptors outlive this call; they are owned by `self`.
                PollFd::new(
                    unsafe { std::os::fd::BorrowedFd::borrow_raw(*fd) },
                    PollFlags::POLLIN,
                )
            })
            .collect();

        let millis: u16 = timeout.min(u16::MAX as u64) as u16;
        match nix::poll::poll(&mut poll_fds, PollTimeout::from(millis)) {
            Ok(0) => return ReadOutcome::Idle,
            Ok(_) => {}
            Err(Errno::EINTR) => return ReadOutcome::Idle,
            Err(e) => return ReadOutcome::Failed(std::io::Error::from(e).kind()),
        }

        for (i, (stream, fd)) in fds.iter().enumerate() {
            let Some(revents) = poll_fds[i].revents() else {
                continue;
            };
            if !revents.intersects(PollFlags::POLLIN | PollFlags::POLLHUP) {
                continue;
            }
            let mut buf = [0u8; 65536];
            match nix::unistd::read(
                // SAFETY: as above.
                unsafe { std::os::fd::BorrowedFd::borrow_raw(*fd) },
                &mut buf,
            ) {
                Ok(0) => {
                    // End of this descriptor. A terminal reports EIO instead, handled below.
                    self.retire(*stream);
                    if self.stdout.is_none() && self.stderr.is_none() {
                        return ReadOutcome::Ended;
                    }
                    return ReadOutcome::Idle;
                }
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    return ReadOutcome::Bytes {
                        stream: *stream,
                        len: n,
                    };
                }
                // A pseudo-terminal whose child has gone answers EIO rather than zero. It is an
                // ordinary end of output, not a failure, and treating it as one would report a
                // descriptor error every time a task finished.
                Err(Errno::EIO) => {
                    self.retire(*stream);
                    if self.stdout.is_none() && self.stderr.is_none() {
                        return ReadOutcome::Ended;
                    }
                    return ReadOutcome::Idle;
                }
                Err(Errno::EINTR) | Err(Errno::EAGAIN) => return ReadOutcome::Idle,
                Err(e) => return ReadOutcome::Failed(std::io::Error::from(e).kind()),
            }
        }
        ReadOutcome::Idle
    }
}

impl PtyOutput {
    fn retire(&mut self, stream: Stream) {
        match stream {
            Stream::Stdout => self.stdout = None,
            Stream::Stderr => self.stderr = None,
        }
    }
}

/// The controlling half. Shared, so a keystroke never waits behind a blocked read.
struct PtyControl {
    shared: Arc<Shared>,
}

impl TaskControl for PtyControl {
    fn write_stdin(&self, data: &[u8]) -> Result<(), ControlError> {
        let guard = self.shared.input.lock().expect("input lock");
        let Some(fd) = *guard else {
            return Err(ControlError::Gone);
        };
        // SAFETY: the descriptor is owned for the task's lifetime.
        let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
        let mut written = 0usize;
        while written < data.len() {
            match nix::unistd::write(borrowed, &data[written..]) {
                Ok(0) => return Err(ControlError::Gone),
                Ok(n) => written += n,
                Err(Errno::EINTR) => continue,
                Err(e) => return Err(errno_to_control(e)),
            }
        }
        Ok(())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), ControlError> {
        let guard = self.shared.input.lock().expect("input lock");
        let Some(fd) = *guard else {
            return Err(ControlError::Gone);
        };
        let size = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // A task started with pipes has no terminal, so this fails and is **ignored** by the
        // caller rather than surfaced: `resizePty` is a notification and has no way to refuse.
        let rc =
            unsafe { nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ as nix::libc::c_ulong, &size) };
        if rc == 0 {
            Ok(())
        } else {
            Err(ControlError::Failed(std::io::ErrorKind::Unsupported))
        }
    }

    fn signal(&self, signal: TaskSignal) -> Result<(), ControlError> {
        let sig = match signal {
            TaskSignal::Int => Signal::SIGINT,
            TaskSignal::Term => Signal::SIGTERM,
            TaskSignal::Kill => Signal::SIGKILL,
        };
        // To the **group**, never the bare pid. A shell's children go with it, which is what
        // makes SC-027's "at any depth" true without a cgroup.
        killpg(self.shared.child, sig).map_err(errno_to_control)
    }

    fn reap(&self) -> Option<Exit> {
        if let Some(exit) = *self.shared.exit.lock().expect("exit lock") {
            return Some(exit);
        }
        match waitpid(self.shared.child, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(_, code)) => {
                let exit = Exit::Code(code);
                *self.shared.exit.lock().expect("exit lock") = Some(exit);
                Some(exit)
            }
            Ok(WaitStatus::Signaled(_, sig, _)) => {
                // The raw number, because what kills a task is the host's whole vocabulary and
                // not the three this feature sends.
                let exit = Exit::Signal(sig as i32);
                *self.shared.exit.lock().expect("exit lock") = Some(exit);
                Some(exit)
            }
            _ => None,
        }
    }
}

impl Drop for PtyControl {
    fn drop(&mut self) {
        if let Some(fd) = self.shared.input.lock().expect("input lock").take() {
            // SAFETY: taken from the slot, so nothing else will close it.
            let _ = close(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) });
        }
    }
}

use std::os::fd::FromRawFd;
