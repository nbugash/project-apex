//! `ide-engine` — the remote half of the product.
//!
//! This file is the composition root and the stdio loop, and nothing else. The behaviour lives in
//! the library beside it: dispatch is an inbound adapter, the filesystem is an outbound port, and
//! the workspace rules are use cases (Principle VIII).

use apex_engine::adapters::inbound::rpc::{self, Action};
use apex_engine::adapters::outbound::engine_socket::{self, Claim};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::adapters::outbound::watchers::{WatcherFactory, Watchers};
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::{FrameCodec, FrameError};
use bytes::BytesMut;
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;

fn main() {
    // **Who is the engine, decided first.** A process that turns out to be a proxy has nothing to
    // compose: it builds no watcher, starts no task service, and holds no state. Deciding after
    // composing would mean a second process creating an inotify handle and a clock it is about to
    // throw away (A-ENGINELIFE rule 1).
    let socket = engine_socket::default_path();
    let claimed = match engine_socket::claim(&socket) {
        Ok(claimed) => claimed,
        Err(e) => {
            // A socket that can be neither bound nor connected to is a misconfigured host, and
            // saying so is more use than starting a second engine that nobody can reach.
            eprintln!(
                "cannot reach or become the engine at {}: {e}",
                socket.display()
            );
            std::process::exit(1);
        }
    };
    if let Claim::Proxy(stream) = claimed {
        proxy(stream);
        return;
    }

    // The composition root: every dependency is constructed here and handed inward. No hidden
    // singletons, no service locator.
    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    let registry = SessionRegistry::new();
    let mut codec = FrameCodec::new();
    let mut stdin = std::io::stdin();
    // One writer, shared. The stdio loop is no longer the only thing that speaks: the watcher
    // thread F004 adds is the second, and §4.6 makes this one pipe and one queue.
    let writer = Arc::new(FrameWriter::to_stdout());

    // How a watcher is made for one workspace. A closure so this file decides, and so the
    // engine still builds and runs on a host with no inotify -- where the factory yields
    // nothing, `workspace/watch` is refused, and FR-027's degradation applies: browsing and
    // reading continue, and the loss is stated rather than silent.
    // Linux gets a watcher; anything else gets none, and `workspace/watch` is refused with a
    // reason rather than appearing to succeed (FR-027, A-WATCHLOCAL). The concrete library is
    // named only inside its own adapter, which is what `inotify_confinement.rs` enforces.
    #[cfg(target_os = "linux")]
    let factory: WatcherFactory = apex_engine::adapters::outbound::inotify_watcher::factory();
    #[cfg(not(target_os = "linux"))]
    let factory: WatcherFactory = Box::new(|_root| None);
    let watchers = Watchers::new(factory, Arc::clone(&fs), Arc::clone(&writer), codec.clone());

    // One clock, shared. `Arc` rather than `Box` because the chunker reads it on every reader
    // thread while the escalation thread and the stop path read it too -- a `Box` can be handed
    // to exactly one of them.
    let clock: Arc<dyn apex_engine::application::ports::clock::Clock> =
        Arc::new(apex_engine::adapters::outbound::system_clock::SystemClock);

    // How a task is run. The pty factory lives **inside** its adapter, on the terms the watcher
    // factory does: Linux gets a real runner, anything else gets none, and `execution/runTask`
    // is refused with a reason rather than appearing to succeed.
    #[cfg(target_os = "linux")]
    let tasks = Some(TaskService::new(
        Arc::clone(&writer),
        Arc::clone(&clock),
        Arc::new(apex_engine::adapters::outbound::pty_runner::PtyRunner::new()),
    ));
    #[cfg(not(target_os = "linux"))]
    let tasks: Option<TaskService> = None;

    // A restart is announced, never inferred. The client learns about it because it was told,
    // and the identity it carries is what distinguishes a restart from a new session.
    if registry.restarted() {
        let notice = registry.restart_notice();
        if let Some(frame) = rpc::encode_notification(&codec, "session/onRestart", &notice) {
            let _ = writer.write_interactive(&frame);
        }
    }

    // This process's own stdio is the first client. Serving it is the ordinary case, and
    // everything after it is A-ENGINELIFE's: the client goes away, the engine does not.
    serve(
        &mut stdin,
        &registry,
        &roots,
        fs.as_ref(),
        &watchers,
        tasks.as_ref(),
        &mut codec,
        &writer,
    );

    // The client went away. Under A-TASKLIFE that is not a reason to stop anything, so what
    // follows waits for it to come back -- and exits only when there is nothing left to come back
    // to.
    let listener = match claimed {
        Claim::Bound(listener) => listener,
        // Unreachable: a proxy returned above. Kept as an arm rather than an `unwrap` because a
        // panic in the composition root is the worst place for one.
        Claim::Proxy(_) => return,
    };
    if listener.set_nonblocking(true).is_err() {
        // Without a non-blocking listener the wait below cannot notice the last task ending, and
        // the engine would linger for as long as nobody reconnected. Better to exit now.
        engine_socket::release(&socket);
        return;
    }

    loop {
        registry.set_attached(false);

        // **Exit when there is nothing to preserve** (A-ENGINELIFE rule 3). Checked here and on
        // every tick below, because the last task can end while this is waiting: a check made
        // only once would leave an engine waiting forever for a client with nothing to show it.
        let waiting_for = match accept_while_tasks_remain(&listener, tasks.as_ref()) {
            Some(stream) => stream,
            None => {
                engine_socket::release(&socket);
                return;
            }
        };

        // **The newest client wins** (A-ENGINELIFE rule 2). A laptop that changed networks leaves
        // a half-dead channel the far end cannot tell from a live one, so a new connection
        // displaces whatever was there rather than being refused.
        let Ok(outbound) = waiting_for.try_clone() else {
            continue;
        };
        writer.retarget(Box::new(outbound));
        registry.set_attached(true);
        // A reconnecting client starts a fresh conversation, so anything half-read from the
        // previous one is discarded: a partial frame from a connection that is gone can only
        // corrupt the first frame of the one that replaced it.
        codec = FrameCodec::new();

        let mut inbound = waiting_for;
        serve(
            &mut inbound,
            &registry,
            &roots,
            fs.as_ref(),
            &watchers,
            tasks.as_ref(),
            &mut codec,
            &writer,
        );
    }
}

/// Wait for a client, giving up when the engine has nothing left to preserve.
///
/// Polled rather than blocking, so the last task ending is noticed. `accept` on a blocking
/// listener would hold this thread until somebody connected, which for an engine whose tasks have
/// all finished is forever.
fn accept_while_tasks_remain(
    listener: &UnixListener,
    tasks: Option<&TaskService>,
) -> Option<UnixStream> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Blocking again for the conversation itself: the dispatch loop is a reader, and
                // a non-blocking read would turn it into a spin.
                let _ = stream.set_nonblocking(false);
                return Some(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if tasks.map_or(0, TaskService::live) == 0 {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            Err(_) => return None,
        }
    }
}

/// Read frames from one client until it goes away.
///
/// Returns when the connection closes, which is a fact about the connection and not about the
/// engine -- everything it was doing keeps going.
#[allow(clippy::too_many_arguments)]
fn serve(
    input: &mut dyn Read,
    registry: &SessionRegistry,
    roots: &InMemoryRoots,
    fs: &dyn FileSystem,
    watchers: &Watchers,
    tasks: Option<&TaskService>,
    codec: &mut FrameCodec,
    writer: &Arc<FrameWriter>,
) {
    let mut buf = BytesMut::new();
    let mut chunk = [0u8; 8192];
    loop {
        match input.read(&mut chunk) {
            Ok(0) | Err(_) => return, // the client went away
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        loop {
            match codec.decode(&mut buf) {
                Ok(Some(frame)) => {
                    match rpc::dispatch(registry, roots, fs, Some(watchers), tasks, codec, &frame.0)
                    {
                        Action::Reply(reply) => {
                            let _ = writer.write_interactive(&reply);
                        }
                        Action::Nothing => {}
                        Action::Restart(ack) => {
                            rpc::drain_and_exec(registry, codec, &mut buf, &ack);
                        }
                    }
                }
                Ok(None) => break,
                // A refused frame is refused alone. The codec has already left the buffer at a
                // boundary, so the next frame still reads.
                Err(FrameError::TooLarge(n)) => {
                    eprintln!("refused a frame declaring {n} bytes");
                }
                Err(FrameError::Malformed(why)) => {
                    eprintln!("discarded a frame: {why}");
                }
            }
        }
    }
}

/// Be the pipe between this process's stdio and the engine that already holds the socket.
///
/// **No framing, no parsing, no buffering decisions.** A proxy that understood the protocol would
/// be a second place the protocol is implemented, and the two would drift -- the first time a
/// frame's shape changed, a client reaching the engine directly and a client reaching it through
/// here would see different behaviour, and only one of them would be a bug anybody could find.
fn proxy(stream: UnixStream) {
    let Ok(mut to_engine) = stream.try_clone() else {
        return;
    };
    let mut from_engine = stream;

    // Both directions at once. One thread doing them in turn would deadlock the first time each
    // side was waiting for the other, which for an interactive protocol is immediately.
    let up = std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin(), &mut to_engine);
        // Tell the engine this end is finished, or it waits for a client that has gone.
        let _ = to_engine.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut from_engine, &mut std::io::stdout());
    // Either side closing ends the proxy. Waiting for both would hold a process open because the
    // half nobody is using has not noticed yet.
    drop(from_engine);
    let _ = up.join();
}
