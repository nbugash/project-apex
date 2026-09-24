//! Reports its working directory, a chosen environment variable, and whether `PATH` survived.
//!
//! US1.4 and US1.5 had no observable without this. The `PATH` line is the one that matters:
//! §4.8 says a caller's `env` is merged **over** the engine's inherited environment rather than
//! replacing it, and an implementation that replaces it passes every other test here, because
//! the fixtures are spawned by absolute path and never need `PATH` to run at all.

use std::io::Write;

fn main() {
    let mut out = std::io::stdout();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("<error: {e}>"));

    writeln!(out, "CWD {cwd}").expect("write");
    writeln!(
        out,
        "APEX_FIXTURE_KEY {}",
        std::env::var("APEX_FIXTURE_KEY").unwrap_or_else(|_| "<unset>".into())
    )
    .expect("write");
    writeln!(
        out,
        "PATH {}",
        if std::env::var_os("PATH").is_some() {
            "set"
        } else {
            "unset"
        }
    )
    .expect("write");
    out.flush().expect("flush");
}
