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
//!   malformed         reply with a body that is not JSON
//!   oversized         declare a length beyond the cap
//!   stall=<ms>        stop answering, then close after <ms> — how `ssh` behaves when its
//!                     keepalive gives up, which is the only way a silent network death
//!                     becomes observable to the transport
//!   close-mid-frame   write half a frame and exit
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
    malformed: bool,
    oversized: bool,
    stall_ms: Option<u64>,
    close_mid_frame: bool,
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
                _ => match part {
                    "malformed" => s.malformed = true,
                    "oversized" => s.oversized = true,
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

    let mut stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut seen = 0usize;

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

            if script.oversized {
                // A declared length beyond the cap, with no body to match it. The transport
                // must refuse this before allocating.
                let _ = out.write_all(format!("{HEADER}{}\r\n\r\n", CAP + 1).as_bytes());
                let _ = out.flush();
                continue;
            }
            if script.malformed {
                let junk = "this is not json";
                let _ = out.write_all(format!("{HEADER}{}\r\n\r\n{junk}", junk.len()).as_bytes());
                let _ = out.flush();
                continue;
            }

            let reply = format!(r#"{{"jsonrpc":"2.0","id":"{id}","result":{{"echo":true}}}}"#);
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
