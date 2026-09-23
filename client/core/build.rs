use std::path::PathBuf;

/// Where the engine build leaves its binary. Overridable so a release build can point at a
/// cross-compiled artifact rather than the host-native one.
fn engine_artifact() -> PathBuf {
    if let Ok(p) = std::env::var("APEX_ENGINE_BIN") {
        return PathBuf::from(p);
    }
    // The workspace target directory, three levels up from client/core.
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join(profile)
        .join("ide-engine")
}

fn main() {
    // Embed the engine the client will deploy, and its digest, computed here from the same
    // bytes. A hand-maintained hash is a hash that is eventually wrong.
    //
    // A missing artifact is recorded rather than fatal. `build.rs` runs before Cargo has built
    // the engine member, so failing here would break every unrelated `cargo test` on a fresh
    // clone — including F001's suite, which has nothing to do with the engine. The absence is
    // caught by `the_engine_artifact_is_embedded` instead, which names the command to run.
    let artifact = engine_artifact();
    println!("cargo:rerun-if-changed={}", artifact.display());
    println!("cargo:rerun-if-env-changed=APEX_ENGINE_BIN");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    match std::fs::read(&artifact) {
        Ok(bytes) => {
            let digest = sha256_hex(&bytes);
            std::fs::write(out.join("engine_artifact.bin"), &bytes).expect("stage artifact");
            std::fs::write(
                out.join("engine_artifact.rs"),
                format!(
                    "pub const ENGINE_BYTES: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/engine_artifact.bin\"));\n\
                     pub const ENGINE_DIGEST: Option<&str> = Some(\"{digest}\");\n"
                ),
            )
            .expect("write artifact module");
        }
        Err(_) => {
            std::fs::write(
                out.join("engine_artifact.rs"),
                "pub const ENGINE_BYTES: &[u8] = &[];\n\
                 pub const ENGINE_DIGEST: Option<&str> = None;\n",
            )
            .expect("write artifact module");
        }
    }

    tauri_build::build()
}

/// SHA-256, implemented here rather than pulled in as a build dependency.
///
/// The client verifies against `sha256sum` on the remote host, so the algorithm is fixed by
/// what exists there (see research.md). This is the only place the client computes one, and a
/// build script that drags in a dependency tree to hash one file is a build script that slows
/// every compile.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());

    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (i, v) in [a, b, c, d, e, f, g, hh].iter().enumerate() {
            h[i] = h[i].wrapping_add(*v);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}
