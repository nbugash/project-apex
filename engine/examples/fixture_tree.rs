//! A three-level process tree: this process, a child, and a grandchild.
//!
//! Three levels and not two. SC-012 counts the named process and its direct children; SC-027
//! counts arbitrary depth. A two-level fixture satisfies both, so quickstart §10's mutation --
//! signal the pid rather than the group -- would pass against it and the pair would be
//! measuring one thing twice.
//!
//! No process here installs a signal handler. Children do not inherit a signal sent to their
//! parent, so outliving it is the default behaviour; only something that signals the process
//! **group** reaches all three.

use std::io::Write;

fn main() {
    let depth: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(0);

    let mut out = std::io::stdout();
    writeln!(out, "LEVEL {depth} pid={}", std::process::id()).expect("write");
    out.flush().expect("flush");

    if depth < 2 {
        let exe = std::env::current_exe().expect("current_exe");
        // Deliberately never waited on. `clippy::zombie_processes` is right in general and
        // wrong here: the child has to outlive this process for the fixture to mean anything,
        // because what is being tested is whether stopping the parent reaches the tree. A
        // parent that reaped its child would make SC-027 pass against an implementation that
        // signals the pid and never touches the group.
        #[allow(clippy::zombie_processes)]
        let _child = std::process::Command::new(exe)
            .arg((depth + 1).to_string())
            .spawn()
            .expect("spawn");
    }

    // Outlive any reasonable test. Nothing here exits on its own: the test is what ends it,
    // and a fixture that timed out would make "zero processes running" true for the wrong reason.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
