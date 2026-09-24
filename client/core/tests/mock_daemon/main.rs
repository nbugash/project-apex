//! A stand-in for the remote engine.
//!
//! It speaks `Content-Length` framing and **nothing above it**. It implements no method from
//! the §4.8 catalogue, deliberately: giving it application behaviour would make it a second
//! implementation of the engine, which would then drift from the real one, and whose
//! divergence F002 would discover.
//!
//! Restricting it to framing bounds that risk — framing is the layer §4.1 defines normatively
//! and exactly, so the mock and the engine can be checked against the same text.
//!
//! Behaviour is scripted through `APEX_MOCK_SCRIPT`, a comma-separated list:
//!
//!   echo              reply to every request (the default)
//!   delay=<ms>        wait before each reply
//!   drop=<n>          silently drop every nth reply
//!   malformed[=<n>]   reply with a body that is not JSON — every reply, or only the nth
//!   oversized[=<n>]   declare a length beyond the cap — every reply, or only the nth
//!   stall=<ms>        stop answering, then close after <ms> — how `ssh` behaves when its
//!                     keepalive gives up, which is the only way a silent network death
//!                     becomes observable to the transport
//!   close-mid-frame   write half a frame and exit
//!   reorder=<n>       hold n replies, then emit them in reverse — a work pool finishing
//!                     out of order, which is the ordinary case for a real engine and the
//!                     one correlation exists for
//!
//! Run with `harness = false`: this is a binary the transport spawns, not a test.

// This process exists to be slow on demand: delays, stalls and lossy links are its
// whole purpose. The crate forbids `std::thread::sleep` to keep the interaction path
// honest, and none of this runs on that path — it is a separate binary with no runtime.
#![allow(clippy::disallowed_methods)]

use std::io::{Read, Write};

const HEADER: &str = "Content-Length: ";
const SEPARATOR: &[u8] = b"\r\n\r\n";
/// Mirrors `domain::request::MAX_FRAME_BYTES`. The mock cannot import the crate under
/// `harness = false`, so this is the one duplicated constant; the oversized case only needs
/// to exceed the cap, not match it.
const CAP: usize = 1024 * 1024;

#[derive(Default, Debug)]
struct Script {
    delay_ms: u64,
    drop_every: usize,
    /// `Some(0)` means every reply; `Some(n)` means the nth only. The numeric form is what
    /// lets a test assert the stream stayed aligned: a hostile frame followed by a normal
    /// request that must still be answered.
    malformed: Option<usize>,
    oversized: Option<usize>,
    stall_ms: Option<u64>,
    close_mid_frame: bool,
    reorder: usize,
    /// Emit one **caller-supplied** frame this many milliseconds in, unprompted.
    ///
    /// The body comes from `APEX_MOCK_FRAME` and this process never looks at it. That is
    /// the whole point: F004 is the first feature whose traffic includes a frame the engine
    /// originates, and a double that knew how to send a named event would be a second
    /// engine. `the_mock_implements_no_engine_method` fails the build if any §4.8 method
    /// name appears in this directory -- it caught the first draft of this very comment --
    /// so the name lives in the calling test's string and the mock carries only the
    /// framing, which is what this file is for.
    notify_ms: Option<u64>,
}

impl Script {
    fn from_env() -> Self {
        let raw = std::env::var("APEX_MOCK_SCRIPT").unwrap_or_default();
        let mut s = Self::default();
        for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some(("delay", v)) => s.delay_ms = v.parse().unwrap_or(0),
                Some(("drop", v)) => s.drop_every = v.parse().unwrap_or(0),
                Some(("stall", v)) => s.stall_ms = Some(v.parse().unwrap_or(0)),
                Some(("reorder", v)) => s.reorder = v.parse().unwrap_or(0),
                Some(("malformed", v)) => s.malformed = Some(v.parse().unwrap_or(0)),
                Some(("oversized", v)) => s.oversized = Some(v.parse().unwrap_or(0)),
                Some(("notify", v)) => s.notify_ms = Some(v.parse().unwrap_or(0)),
                _ => match part {
                    "malformed" => s.malformed = Some(0),
                    "oversized" => s.oversized = Some(0),
                    "close-mid-frame" => s.close_mid_frame = true,
                    // The profile the feature map names: 250 ms round trip, 5% loss.
                    // A shorthand rather than a default, because most tests want a fast
                    // link and only the latency ones want this.
                    "lossy" => {
                        s.delay_ms = 250;
                        s.drop_every = 20;
                    }
                    "echo" => {}
                    other => eprintln!("mock: ignoring unknown directive {other:?}"),
                },
            }
        }
        s
    }
}

fn main() {
    let script = Script::from_env();

    // Stalling is decided before reading anything: the point is to model a link that has
    // gone silent, not one that answers and then stops.
    if let Some(ms) = script.stall_ms {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        // Then close, which is what `ssh` does once ServerAliveCountMax is exceeded. The
        // transport sees EOF; there is no second signal for it to see.
        return;
    }

    // An unprompted frame, on its own thread, so it can arrive while a request is in
    // flight -- which is the case worth exercising and the one a reply-shaped directive
    // cannot produce.
    if let Some(ms) = script.notify_ms {
        if let Ok(frame) = std::env::var("APEX_MOCK_FRAME") {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                // Locked for the whole frame. `Stdout::write_all` may issue several writes,
                // and a frame interleaved with a reply is a corrupt stream rather than a
                // slow one.
                let stdout = std::io::stdout();
                let mut held = stdout.lock();
                let _ =
                    held.write_all(format!("{HEADER}{}\r\n\r\n{frame}", frame.len()).as_bytes());
                let _ = held.flush();
            });
        } else {
            eprintln!("mock: notify= set with no APEX_MOCK_FRAME; nothing to send");
        }
    }

    let mut stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut seen = 0usize;
    // Replies held back when `reorder` is in play.
    let mut held: Vec<String> = Vec::new();

    loop {
        match stdin.read(&mut chunk) {
            Ok(0) => return, // the transport went away
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return,
        }

        while let Some(body) = take_frame(&mut buf) {
            seen += 1;

            if script.drop_every > 0 && seen % script.drop_every == 0 {
                continue; // silently drop: the request will time out, which is the point
            }
            if script.delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(script.delay_ms));
            }

            let id = extract_id(&body).unwrap_or_default();

            if hostile_now(script.oversized, seen) {
                // A declared length beyond the cap, with no body to match it. The transport
                // must refuse this before allocating.
                let _ = out.write_all(format!("{HEADER}{}\r\n\r\n", CAP + 1).as_bytes());
                let _ = out.flush();
                continue;
            }
            if hostile_now(script.malformed, seen) {
                let junk = "this is not json";
                let _ = out.write_all(format!("{HEADER}{}\r\n\r\n{junk}", junk.len()).as_bytes());
                let _ = out.flush();
                continue;
            }

            let reply = reply_for(&id);

            if script.reorder > 0 {
                held.push(reply);
                if held.len() == script.reorder {
                    // Reversed, so the first request asked is the last one answered. If
                    // correlation were positional rather than by id, every outcome would
                    // land on the wrong request and the test would say so.
                    for r in held.drain(..).rev() {
                        let _ = out.write_all(format!("{HEADER}{}\r\n\r\n{r}", r.len()).as_bytes());
                    }
                    let _ = out.flush();
                }
                continue;
            }

            if script.close_mid_frame {
                let whole = format!("{HEADER}{}\r\n\r\n{reply}", reply.len());
                let half = &whole.as_bytes()[..whole.len() / 2];
                let _ = out.write_all(half);
                let _ = out.flush();
                return;
            }
            let _ = out.write_all(format!("{HEADER}{}\r\n\r\n{reply}", reply.len()).as_bytes());
            let _ = out.flush();
        }
    }
}

/// Whether this reply should be hostile: `Some(0)` means always, `Some(n)` the nth only.
fn hostile_now(setting: Option<usize>, seen: usize) -> bool {
    match setting {
        Some(0) => true,
        Some(n) => n == seen,
        None => false,
    }
}

/// The only reply this process knows how to make.
///
/// One shape for every method, whatever was asked. Giving the mock real behaviour would
/// make it a second implementation of the engine, which would drift from the real one — and
/// the drift would be discovered by F002, against a double that had been passing for
/// months. Framing is the layer §4.1 defines normatively and exactly, so a mock restricted
/// to framing can be checked against the same text as the engine.
fn reply_for(id: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":"{id}","result":{{"echo":true}}}}"#)
}

/// Pull one complete frame out of `buf`, leaving the remainder.
fn take_frame(buf: &mut Vec<u8>) -> Option<String> {
    let sep = buf.windows(SEPARATOR.len()).position(|w| w == SEPARATOR)?;
    let header = std::str::from_utf8(&buf[..sep]).ok()?;
    let len: usize = header
        .trim_start()
        .strip_prefix(HEADER)?
        .trim()
        .parse()
        .ok()?;
    let start = sep + SEPARATOR.len();
    if buf.len() < start + len {
        return None;
    }
    let body = String::from_utf8(buf[start..start + len].to_vec()).ok()?;
    buf.drain(..start + len);
    Some(body)
}

/// Read the request id back out so the reply can carry it. Deliberately a string scan rather
/// than a JSON parse: the mock must be able to reply to a frame whose body it does not
/// understand, because understanding bodies is the engine's job, not this one's.
fn extract_id(body: &str) -> Option<String> {
    let at = body.find("\"id\"")?;
    let rest = &body[at + 4..];
    let open = rest.find('"')?;
    let after = &rest[open + 1..];
    let close = after.find('"')?;
    Some(after[..close].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §4.8 catalogue, in halves so that this list is not itself a mention of the
    /// methods the source is scanned for.
    const CATALOGUE: &[(&str, &str)] = &[
        ("auth", "handshake"),
        ("session", "shutdown"),
        ("log", "onMessage"),
        ("workspace", "readDirectory"),
        ("workspace", "stat"),
        ("workspace", "readFile"),
        ("workspace", "writeFile"),
        ("workspace", "createFile"),
        ("workspace", "createDirectory"),
        ("workspace", "rename"),
        ("workspace", "delete"),
        ("workspace", "search"),
        ("workspace", "onFileEvent"),
        ("workspace", "invalidateAll"),
    ];

    /// T073. The mock is a framing double, not a second engine.
    ///
    /// Two assertions, because either alone is weak. The behavioural one shows that a
    /// catalogue method gets the same generic reply as anything else. The structural one
    /// shows that no branch for one exists at all — which is what stops the mock acquiring
    /// engine behaviour a method at a time, each addition reasonable on its own.
    #[test]
    fn the_mock_implements_no_engine_method() {
        for (namespace, method) in CATALOGUE {
            let name = format!("{namespace}/{method}");
            let reply = reply_for("7");
            assert!(
                reply.contains(r#""result":{"echo":true}"#),
                "{name} must get the generic reply, not an implementation of itself"
            );
        }

        let source = include_str!("main.rs");
        for (namespace, method) in CATALOGUE {
            let name = format!("{namespace}/{method}");
            assert!(
                !source.contains(&name),
                "the mock has grown an implementation of {name}; it is a framing double, \
                 and a second engine is what F002 would later discover had drifted"
            );
        }
    }

    #[test]
    fn a_hostile_reply_can_be_scheduled_for_one_request_only() {
        assert!(hostile_now(Some(0), 1), "the bare form is every reply");
        assert!(hostile_now(Some(0), 9));
        assert!(
            hostile_now(Some(2), 2),
            "the numeric form is that reply alone"
        );
        assert!(!hostile_now(Some(2), 1));
        assert!(!hostile_now(Some(2), 3));
        assert!(!hostile_now(None, 1));
    }

    #[test]
    fn a_frame_is_taken_whole_or_not_at_all() {
        let mut buf = b"Content-Length: 5\r\n\r\nhel".to_vec();
        assert_eq!(
            take_frame(&mut buf),
            None,
            "a partial body must not be taken"
        );
        buf.extend_from_slice(b"lo");
        assert_eq!(take_frame(&mut buf), Some("hello".to_string()));
        assert!(buf.is_empty());
    }

    #[test]
    fn the_id_is_read_back_so_the_reply_can_carry_it() {
        assert_eq!(
            extract_id(r#"{"jsonrpc":"2.0","id":"42","method":"anything"}"#),
            Some("42".to_string())
        );
        assert_eq!(
            extract_id(r#"{"jsonrpc":"2.0","method":"notification"}"#),
            None
        );
    }
}
