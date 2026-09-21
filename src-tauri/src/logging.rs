//! Structured file logging. No telemetry: `[OPEN: OBS]` has not defined metric names,
//! transport or retention, and Principle IV forbids inventing them here.

use std::io::Write;
use std::sync::OnceLock;
use std::{fs::OpenOptions, path::PathBuf, sync::Mutex};

static SINK: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

pub fn init(path: PathBuf) {
    let _ = SINK.set(Mutex::new(Some(path)));
}

fn write(level: &str, msg: &str) {
    let line = format!("{level} {msg}\n");
    let Some(lock) = SINK.get() else {
        eprint!("{line}");
        return;
    };
    let guard = lock.lock().expect("log sink lock");
    match guard.as_ref() {
        Some(p) => {
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
                let _ = f.write_all(line.as_bytes());
            }
        }
        None => eprint!("{line}"),
    }
}

pub fn info(msg: &str) {
    write("INFO", msg)
}

pub fn warn(msg: &str) {
    write("WARN", msg)
}
