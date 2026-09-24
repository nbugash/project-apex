//! `ide-engine` — the remote half of the product.
//!
//! This file is the composition root and the stdio loop, and nothing else. The behaviour lives in
//! the library beside it: dispatch is an inbound adapter, the filesystem is an outbound port, and
//! the workspace rules are use cases (Principle VIII).

use apex_engine::adapters::inbound::rpc::{self, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::{FrameCodec, FrameError};
use bytes::BytesMut;
use std::io::Read;
use std::sync::Arc;

fn main() {
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

    // A restart is announced, never inferred. The client learns about it because it was told,
    // and the identity it carries is what distinguishes a restart from a new session.
    if registry.restarted() {
        let notice = registry.restart_notice();
        if let Some(frame) = rpc::encode_notification(&codec, "session/onRestart", &notice) {
            let _ = writer.write(&frame);
        }
    }

    let mut buf = BytesMut::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stdin.read(&mut chunk) {
            Ok(0) | Err(_) => return, // the client went away
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        loop {
            match codec.decode(&mut buf) {
                Ok(Some(frame)) => {
                    match rpc::dispatch(&registry, &roots, fs.as_ref(), &codec, &frame.0) {
                        Action::Reply(reply) => {
                            let _ = writer.write(&reply);
                        }
                        Action::Nothing => {}
                        Action::Restart(ack) => {
                            rpc::drain_and_exec(&registry, &mut codec, &mut buf, &ack);
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
