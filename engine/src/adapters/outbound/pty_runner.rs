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
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex};

use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, FdFlag};
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
unsafe fn become_the_child(plan: &ChildPlan, status: RawFd) -> ! {
    // `setsid` has already run in both callers, before the descriptors were arranged. It belongs
    // there rather than here because the terminal path has to acquire its controlling terminal
    // between the two, and that acquisition only works for a session leader.

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
        report_and_exit(status, *nix::libc::__errno_location());
    }

    // `execvpe` and not `execvp`. The latter takes no environment and the child would inherit
    // this process's, silently discarding the set the use case merged -- which is FR-001's
    // environment and SC-025's redaction both quietly not happening.
    let _ = execvpe(&plan.program, &plan.argv, &plan.envp);
    // `exec` only returns on failure. The parent learns **which** failure through the status
    // pipe rather than from the exit code, because an exit code cannot distinguish "there is no
    // such command" from "the command ran and chose to exit 127" -- and the two lead to opposite
    // things being said to the developer (SC-015, §4.4's -32011).
    report_and_exit(status, *nix::libc::__errno_location());
}

/// Tell the parent why `exec` did not happen, then leave.
///
/// # Safety
/// Called only in the child of a `fork`, after `exec` has failed. `write` and `_exit` are
/// async-signal-safe; nothing here allocates or takes a lock.
unsafe fn report_and_exit(status: RawFd, errno: i32) -> ! {
    let bytes = errno.to_ne_bytes();
    // One write of four bytes to a pipe is atomic, so the parent either reads the whole errno or
    // reads nothing. A short write would be indistinguishable from a successful exec.
    let _ = nix::libc::write(status, bytes.as_ptr() as *const nix::libc::c_void, 4);
    nix::libc::_exit(127);
}

/// Wait for the child to say whether `exec` happened.
///
/// A successful `exec` closes the write end, because it is close-on-exec, and this reads end of
/// file. A failed one writes an errno first. So "nothing was written" **is** the success signal,
/// which is why the write end must be closed in the parent first: with it still open here, the
/// read would block forever waiting for a process that has already gone.
fn exec_outcome(status: OwnedFd) -> Result<(), SpawnFailure> {
    let mut buf = [0u8; 4];
    let mut filled = 0;
    while filled < buf.len() {
        match nix::unistd::read(&status, &mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(Errno::EINTR) => continue,
            Err(e) => return Err(errno_to_spawn(e)),
        }
    }
    if filled == 0 {
        return Ok(());
    }
    Err(errno_to_spawn(Errno::from_raw(i32::from_ne_bytes(buf))))
}

/// Mark a descriptor close-on-exec.
///
/// **Every descriptor this adapter creates gets this, and the reason is not tidiness.** `fork`
/// copies the whole descriptor table and `exec` keeps everything not marked close-on-exec, so a
/// task starting while another task is running inherits the other's pty and pipes -- and holds
/// them open for its entire life.
///
/// The consequence is not a leak of a number. The other task's master never reaches end of file,
/// because a descriptor for the far side is still open in a process that has nothing to do with
/// it, so that task's reader never reports `Ended`, its `execution/onExit` is never sent, and its
/// identity is never released (FR-023, SC-014). A build would finish and the panel would sit
/// there waiting for it.
///
/// `dup2` clears the flag on the descriptor it creates, which is why the child's 0, 1 and 2
/// survive `exec` while everything they were copied from does not. That is the whole mechanism:
/// mark everything, dup what the child needs, let `exec` discard the rest.
/// Serialises the whole of creating a task's descriptors and forking.
///
/// Marking a descriptor close-on-exec **after** creating it cannot close a race that exists
/// before the mark. Between `openpty` and `set_cloexec` the descriptor is inheritable, and a fork
/// on another thread in that window produces a child that keeps it across `exec` -- for its whole
/// life. That is not theoretical: it was observed as a task holding `/dev/ptmx`, another task's
/// terminal master, with everything correctly marked.
///
/// The remedy is to make the sequence indivisible rather than to make each step safer. Creating
/// descriptors and forking is rare -- once per task -- so serialising costs nothing measurable,
/// and it removes the whole class at once rather than one descriptor at a time. `O_CLOEXEC` at
/// creation would fix the pipes, and `openpty` offers no way to ask for it.
///
/// Held across the fork, which is safe because the child touches nothing but async-signal-safe
/// calls and then `exec`s: it never takes this lock, so there is no lock to be left held in a
/// child that will not release it.
static SPAWNING: Mutex<()> = Mutex::new(());

/// Take the spawn lock, ignoring poisoning: a panic during a previous spawn says nothing about
/// whether descriptors can be created now, and refusing every later task would turn one failure
/// into a permanent one.
fn spawn_lock() -> std::sync::MutexGuard<'static, ()> {
    SPAWNING.lock().unwrap_or_else(|p| p.into_inner())
}

fn set_cloexec<F: AsFd>(fd: &F) -> Result<(), SpawnFailure> {
    fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
        .map(|_| ())
        .map_err(errno_to_spawn)
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
    let _spawning = spawn_lock();
    let (status_r, status_w) = pipe().map_err(errno_to_spawn)?;
    // The write end must be close-on-exec: that is the whole mechanism. A successful `exec`
    // closes it, the parent reads end of file, and "nothing was written" means the command ran.
    set_cloexec(&status_w)?;
    set_cloexec(&status_r)?;
    let pair = openpty(Some(&size), None).map_err(errno_to_spawn)?;
    let (master, slave) = (pair.master, pair.slave);
    // Both ends, before the fork. The child clears the flag on 0, 1 and 2 by dup2-ing onto them.
    set_cloexec(&master)?;
    set_cloexec(&slave)?;

    // SAFETY: the child path below calls only async-signal-safe functions and never allocates.
    match unsafe { fork() }.map_err(errno_to_spawn)? {
        ForkResult::Child => {
            // `setsid`, then the descriptors, then `TIOCSCTTY`. **The order is the whole of it**,
            // and it is the sequence `login_tty` performs.
            //
            // `TIOCSCTTY` succeeds only for a session leader that has no controlling terminal
            // yet. Called before `setsid` it fails with EPERM -- the child is still in the
            // engine's session -- and the failure is invisible, because the ioctl's result is
            // discarded and everything afterwards still works: the process runs, output flows,
            // and `isatty` answers true, since that asks whether a descriptor is *a* terminal
            // and not whether it is *this process's* terminal.
            //
            // What silently does not exist is job control. The line discipline generates SIGINT
            // for the **foreground process group of the terminal**, and a terminal nobody
            // claimed has no such group, so a `0x03` is echoed as `^C` and interrupts nothing.
            // That is US2.2 and SC-008 failing while every other assertion about a terminal
            // passes, which is how this survived until a test wrote the byte and waited for the
            // signal.
            //
            // Its own session also gives the task its own process group, so a stop reaches
            // everything it spawned and never reaches the engine (FR-018, FR-006a).
            let _ = setsid();
            // One device: the terminal is stdin, stdout and stderr at once, which is why a
            // `pty: true` task's streams arrive merged (A-TASKSTREAM).
            let _ = dup2_stdin(&slave);
            let _ = dup2_stdout(&slave);
            let _ = dup2_stderr(&slave);
            let status = status_w.as_raw_fd();
            unsafe {
                nix::libc::ioctl(0, nix::libc::TIOCSCTTY as nix::libc::c_ulong, 0);
                become_the_child(plan, status)
            }
        }
        ForkResult::Parent { child } => {
            drop(slave);
            // Before reading, or the read waits on a descriptor this process is holding open.
            drop(status_w);
            exec_outcome(status_r)?;
            // **A descriptor of its own for each half.**
            //
            // The two halves of a task are dropped independently -- the reader ends when the
            // task does, the control lives as long as anything might still write to it -- and
            // both close what they hold. Handing them the same descriptor number, one as an
            // `OwnedFd` and one as a raw copy, closes it twice.
            //
            // A double close is not a leak, which is what makes it dangerous. The number is
            // returned to the process after the first close, another thread's `open` takes it,
            // and the second close then shuts *that* down. The engine's own descriptors are in
            // that pool: the client's stdin and stdout, the watcher's inotify handle, every
            // other task's terminal. It surfaced here as `closedir: Bad file descriptor` in an
            // unrelated test, which is what this class of bug looks like from the outside.
            //
            // `F_DUPFD_CLOEXEC` rather than `dup`, because `dup` does not copy the close-on-exec
            // flag and the copy would be inherited by every task started afterwards.
            let input = fcntl(&master, FcntlArg::F_DUPFD_CLOEXEC(0)).map_err(errno_to_spawn)?;
            let shared = Arc::new(Shared::new(child, Some(input)));
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
    let _spawning = spawn_lock();
    let (status_r, status_w) = pipe().map_err(errno_to_spawn)?;
    set_cloexec(&status_w)?;
    set_cloexec(&status_r)?;
    let (stdin_r, stdin_w) = pipe().map_err(errno_to_spawn)?;
    let (stdout_r, stdout_w) = pipe().map_err(errno_to_spawn)?;
    let (stderr_r, stderr_w) = pipe().map_err(errno_to_spawn)?;
    // Six ends, all marked, for the reason `set_cloexec` gives. A concurrently starting task
    // holding the writing end of this task's stdout is a task whose output never ends.
    set_cloexec(&stdin_r)?;
    set_cloexec(&stdin_w)?;
    set_cloexec(&stdout_r)?;
    set_cloexec(&stdout_w)?;
    set_cloexec(&stderr_r)?;
    set_cloexec(&stderr_w)?;

    // SAFETY: as above.
    match unsafe { fork() }.map_err(errno_to_spawn)? {
        ForkResult::Child => {
            // Its own session, and so its own process group: a stop reaches the task and
            // everything it spawned, and never reaches the engine (FR-018, FR-006a). No
            // controlling terminal is acquired, because there is no terminal -- which is also
            // why a `pty: false` task cannot be interrupted by a keystroke and needs
            // `execution/terminate` instead.
            let _ = setsid();
            let _ = dup2_stdin(&stdin_r);
            let _ = dup2_stdout(&stdout_w);
            let _ = dup2_stderr(&stderr_w);
            let status = status_w.as_raw_fd();
            unsafe { become_the_child(plan, status) }
        }
        ForkResult::Parent { child } => {
            drop(stdin_r);
            drop(stdout_w);
            drop(stderr_w);
            drop(status_w);
            exec_outcome(status_r)?;
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
        // A task whose exit has already been observed is not signalled again.
        //
        // Not tidiness: the process is gone and its pid is free for the kernel to hand out, so a
        // signal sent now is a signal to whatever holds that number next -- and it goes to the
        // whole group, which makes it worse. FR-019's window is exactly this moment, between the
        // exit being observed and being delivered, and a client that presses stop inside it must
        // get a success without anything being signalled.
        //
        // `Gone` rather than `Ok`: the caller ignores it, so `terminate` still answers success,
        // and a caller that ever wants to tell "nothing to do" from "done" can.
        if self.shared.exit.lock().expect("exit lock").is_some() {
            return Err(ControlError::Gone);
        }
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
            // SAFETY: this slot is the sole owner of the descriptor in it, and `take` leaves
            // nothing behind for a second drop to close. Sole ownership is established at the
            // two call sites: the pipe path hands over the writing end with `mem::forget`, and
            // the terminal path stores a `F_DUPFD_CLOEXEC` copy rather than the master itself,
            // which the output half owns. Storing the master's own number here as well would
            // close it twice -- see the comment at that call site for why that is worse than a
            // leak.
            let _ = close(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) });
        }
    }
}

use std::os::fd::FromRawFd;
