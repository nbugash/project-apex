//! The helper OpenSSH executes when it needs a passphrase (§3.3).
//!
//! Deliberately tiny. It carries no judgement: it reads the prompt OpenSSH gave it, asks the
//! running application over a local socket, writes the answer to stdout, and zeroes its
//! buffer. Every decision about what to ask, whether to retry and what to do with a refusal
//! belongs in the app, which can see the rest of the connection attempt.
//!
//! Three things make this work, and all three are easy to get wrong:
//!
//! - `SSH_ASKPASS` must be an **absolute** path. OpenSSH execs it with an unpredictable
//!   working directory, so a relative path fails exactly when a user needs the prompt.
//! - `SSH_ASKPASS_REQUIRE=force` must be set, or OpenSSH consults this only when it finds no
//!   tty, and that varies by platform and by `DISPLAY`.
//! - The passphrase crosses a process boundary through a pipe. It is never logged, and the
//!   buffer is zeroed before exit.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

/// Where the running application listens. Passed in rather than discovered, so a helper
/// launched by some other process cannot be answered by ours.
const SOCKET_ENV: &str = "APEX_ASKPASS_SOCKET";

fn main() {
    // OpenSSH passes its own prompt text as the first argument.
    let prompt = std::env::args().nth(1).unwrap_or_default();

    let socket = match std::env::var(SOCKET_ENV) {
        Ok(path) => path,
        // No socket means nobody asked for this prompt. Exiting non-zero tells OpenSSH the
        // credential could not be obtained, which is the truth.
        Err(_) => {
            eprintln!("apex-askpass: {SOCKET_ENV} is not set; refusing to prompt");
            std::process::exit(1);
        }
    };

    let mut answer = match ask(&socket, &prompt) {
        Ok(a) => a,
        Err(e) => {
            // The error must not carry the prompt or anything the app said back.
            eprintln!("apex-askpass: could not reach the application: {e}");
            std::process::exit(1);
        }
    };

    let mut out = std::io::stdout();
    let wrote = out.write_all(&answer).and_then(|()| out.write_all(b"\n"));
    let flushed = out.flush();

    // Zero before exiting, whatever happened. `write_volatile` so this cannot be optimised
    // away as a write nothing reads.
    for byte in answer.iter_mut() {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }

    if wrote.is_err() || flushed.is_err() {
        std::process::exit(1);
    }
}

fn ask(socket: &str, prompt: &str) -> std::io::Result<Vec<u8>> {
    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(prompt.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut answer = Vec::new();
    stream.read_to_end(&mut answer)?;
    while answer.last() == Some(&b'\n') {
        answer.pop();
    }
    Ok(answer)
}
