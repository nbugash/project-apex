//! US5 scenario 2, SC-009 and FR-030, **asserted and printed** (Principle V): an interactive action taken while
//! prefetch runs is no slower than the same action with prefetch idle, within 10%.
//!
//! **Against the real engine, over the real transport.** The cost prefetch imposes on this link is
//! not its request frames, which are small, but the large responses the engine sends back on the
//! same pipe -- so a measurement against the send queue alone would answer a question nobody asked.
//! The engine is the same binary the application runs, spawned as a local child
//! (`LocalEngineSpawner`), as `engine_notifications.rs` does.
//!
//! **What "prefetch running" is here.** Prefetch's own shape: one background-priority read at a
//! time through `RemoteWorkspaceProvider::background`, back to back, over files large enough that
//! each response is a substantial share of a frame. The interactive action is an ordinary read of a
//! small file at interactive priority, the editor's shape.
//!
//! **The p99, against idle plus one prefetch response.** The claim is a bound on the worst wait an
//! interactive reply can suffer -- behind at most one prefetch response, because the client sends
//! interactive first and a frame in flight completes (§4.6) -- so it is the tail that is bounded,
//! not the median. Measured, the median barely moves even with whole-file reads; the tail is what
//! whole-file reads ruin and what chunking fixes. The median is printed as well.

use apex_shell::adapters::outbound::local_engine::LocalEngineSpawner;
use apex_shell::adapters::outbound::openssh::SshTransport;
use apex_shell::adapters::outbound::remote_workspace::RemoteWorkspaceProvider;
use apex_shell::adapters::outbound::transport_sender::TransportSender;
use apex_shell::application::ports::spawner::SpawnSpec;
use apex_shell::application::ports::transport::{Request, RequestTransport};
use apex_shell::application::ports::workspace_provider::WorkspaceProvider;
use apex_shell::application::use_cases::prefetch::PREFETCH_CHUNK_BYTES;
use apex_shell::domain::request::RequestOutcome;
use apex_shell::domain::workspace::{ByteRange, RelPath, WorkspaceId};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

const SAMPLES: usize = 200;
const LARGE_FILES: usize = 32;
/// Under `MAX_INLINE_READ`, so each is one inline response: about 340 KiB on the wire once encoded.
const LARGE_BYTES: usize = 256 * 1024;

fn engine_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/ide-engine")
}

fn median(mut us: Vec<u128>) -> u128 {
    us.sort_unstable();
    us[us.len() / 2]
}

fn p99(mut us: Vec<u128>) -> u128 {
    us.sort_unstable();
    us[(us.len() as f64 * 0.99) as usize - 1]
}

async fn sample(reader: &RemoteWorkspaceProvider, ws: &WorkspaceId, probe: &RelPath) -> Vec<u128> {
    let mut out = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let at = Instant::now();
        reader
            .read_file(ws, probe, None)
            .await
            .expect("the interactive read");
        out.push(at.elapsed().as_micros());
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn interactive_reads_are_not_slowed_by_prefetch() {
    let binary = engine_binary();
    assert!(
        binary.exists(),
        "the engine is not built at {}; run `cargo build -p apex-engine --bins` first",
        binary.display()
    );
    let dir = tempfile::tempdir().expect("temp");
    let root = dir.path().join("ws");
    std::fs::create_dir_all(root.join("big")).expect("dirs");
    std::fs::write(root.join("probe.rs"), "fn probe() {}\n").expect("probe");
    for i in 0..LARGE_FILES {
        std::fs::write(
            root.join(format!("big/{i:02}.txt")),
            vec![b'x'; LARGE_BYTES],
        )
        .expect("big");
    }

    let transport = Arc::new(SshTransport::new(
        Arc::new(LocalEngineSpawner::new(binary)),
        SpawnSpec {
            host: "localhost".into(),
            user: "local".into(),
            assisted: false,
        },
    ));
    transport.connect().expect("the engine did not start");
    let registered = transport
        .send(Request::interactive(
            "workspace/register",
            format!(r#"{{"workspace_id":"ws1","path":"{}"}}"#, root.display()),
        ))
        .await;
    assert!(
        matches!(registered, RequestOutcome::Answered(_)),
        "{registered:?}"
    );

    let sender = Arc::new(TransportSender::new(transport.clone()));
    let interactive = RemoteWorkspaceProvider::new(sender.clone(), None, String::new());
    let background =
        Arc::new(RemoteWorkspaceProvider::new(sender, None, String::new()).background());
    let ws = WorkspaceId("ws1".into());
    let probe = RelPath::parse("/probe.rs").expect("path");

    // Warm both paths once, so neither measurement pays for the first request's setup.
    let _ = sample(&interactive, &ws, &probe).await;
    let idle = sample(&interactive, &ws, &probe).await;
    // What one prefetch response costs on its own: the most an interactive reply may have to wait
    // behind, because the client sends interactive first and a frame in flight completes (§4.6).
    let chunk = ByteRange {
        offset: 0,
        length: PREFETCH_CHUNK_BYTES,
    };
    let big = RelPath::parse("/big/00.txt").expect("path");
    let mut alone = Vec::new();
    for _ in 0..50 {
        let at = Instant::now();
        background
            .read_file(&ws, &big, Some(chunk))
            .await
            .expect("a chunk");
        alone.push(at.elapsed().as_micros());
    }
    let one_chunk = median(alone);

    let stop = Arc::new(AtomicBool::new(false));
    let fetched = Arc::new(AtomicUsize::new(0));
    let prefetch = {
        let (stop, fetched, background, ws) = (
            stop.clone(),
            fetched.clone(),
            background.clone(),
            ws.clone(),
        );
        tokio::spawn(async move {
            // Prefetch's own shape: whole files, one range at a time, back to back.
            let mut i = 0;
            while !stop.load(Ordering::Relaxed) {
                let p = RelPath::parse(&format!("/big/{:02}.txt", i % LARGE_FILES)).expect("path");
                let mut offset = 0;
                while offset < LARGE_BYTES as u64 && !stop.load(Ordering::Relaxed) {
                    let part = background
                        .read_file(
                            &ws,
                            &p,
                            Some(ByteRange {
                                offset,
                                length: PREFETCH_CHUNK_BYTES,
                            }),
                        )
                        .await
                        .expect("a prefetch read");
                    offset += part.bytes.len() as u64;
                    fetched.fetch_add(1, Ordering::Relaxed);
                }
                i += 1;
            }
        })
    };
    // Let the background reads get going before sampling, so the samples really overlap them.
    while fetched.load(Ordering::Relaxed) < 2 {
        tokio::task::yield_now().await;
    }
    // Sampled until the window has genuinely overlapped enough prefetch reads, not for a fixed
    // count: how many a fixed count overlaps depends on the machine, and a comparison that
    // overlapped a handful says little.
    let overlapped_from = fetched.load(Ordering::Relaxed);
    let mut busy = Vec::with_capacity(SAMPLES);
    while busy.len() < SAMPLES || fetched.load(Ordering::Relaxed) - overlapped_from < 20 {
        let at = Instant::now();
        interactive
            .read_file(&ws, &probe, None)
            .await
            .expect("the interactive read");
        busy.push(at.elapsed().as_micros());
    }
    stop.store(true, Ordering::Relaxed);
    prefetch.await.expect("prefetch loop");

    let (idle_median, busy_median) = (median(idle.clone()), median(busy.clone()));
    let (idle_p99, busy_p99) = (p99(idle), p99(busy));
    let bound = (idle_p99 + one_chunk) as f64 * 1.10;
    println!(
        "SC-009 interactive read p99: idle {idle_p99} us, during prefetch {busy_p99} us, bound \
         {bound:.0} us (idle p99 plus one {} KiB prefetch response, {one_chunk} us, within 10%); \
         median idle {idle_median} us, during prefetch {busy_median} us; {} prefetch reads \
         overlapped",
        PREFETCH_CHUNK_BYTES / 1024,
        fetched.load(Ordering::Relaxed),
    );
    // SC-009 as amended. A relative bound on idle alone cannot hold over one pipe; chunking is what
    // makes the added wait one small frame rather than one whole file.
    assert!(
        busy_p99 as f64 <= bound,
        "an interactive read waited longer than one prefetch response: p99 {busy_p99} us against \
         a bound of {bound:.0} us"
    );
    assert!(
        fetched.load(Ordering::Relaxed) >= 20,
        "too few prefetch reads overlapped the samples for the comparison to mean anything"
    );
}
