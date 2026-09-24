//! Allocates until an allocation is refused, and **reports the refusal** before doing anything
//! else.
//!
//! That report is SC-026's observable. An address-space limit does not kill anything: it makes
//! an allocation fail, and what the process does next is the process's own policy. So the
//! criterion measures the refusal, and this fixture makes the refusal visible rather than
//! inferring it from a death.
//!
//! `try_reserve` and not `Vec::reserve`: Rust's default allocation-failure behaviour is to
//! abort the process, which would destroy the observable before it could be printed.
//!
//! The pages are deliberately **not** touched. `RLIMIT_AS` bounds address space, not resident
//! memory, so a reservation alone trips it -- which is what makes this affordable at a 16 GiB
//! limit on any host, rather than requiring sixteen gigabytes of real pages to be written.

use std::io::Write;
use std::time::Instant;

const STEP_BYTES: usize = 16 * 1024 * 1024;

fn main() {
    let mut out = std::io::stdout();
    let started = Instant::now();
    writeln!(out, "ALLOC-BEGIN pid={}", std::process::id()).expect("write");
    out.flush().expect("flush");

    let mut held: Vec<Vec<u8>> = Vec::new();
    loop {
        let mut block: Vec<u8> = Vec::new();
        let request = Instant::now();
        match block.try_reserve_exact(STEP_BYTES) {
            Ok(()) => held.push(block),
            Err(_) => {
                // The refusal's own latency, not the time spent reaching it. SC-026 bounds how
                // long a *request* takes to be denied; how long a process takes to consume its
                // address space is its own business and scales with the limit.
                let refused_in = request.elapsed();
                writeln!(
                    out,
                    "ALLOC-REFUSED refused_us={} reached_after_ms={} held_blocks={}",
                    refused_in.as_micros(),
                    started.elapsed().as_millis(),
                    held.len()
                )
                .expect("write");
                out.flush().expect("flush");
                return;
            }
        }
    }
}
