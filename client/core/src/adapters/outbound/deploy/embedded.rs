//! The engine artifacts the client carries, embedded at build time.
//!
//! `build.rs` writes this module's constants from the same bytes it embeds, so the digest and
//! the artifact cannot disagree — a hand-maintained hash is a hash that is eventually wrong.

include!(concat!(env!("OUT_DIR"), "/engine_artifact.rs"));

/// The host-native artifact, when the build had one to embed.
pub fn host_artifact() -> Option<(&'static [u8], &'static str)> {
    match ENGINE_DIGEST {
        Some(d) if !ENGINE_BYTES.is_empty() => Some((ENGINE_BYTES, d)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The artifact must be there. Absence is a skipped build step, not a broken client, so
    /// this names the step rather than letting the failure surface at deployment against a
    /// host that did nothing wrong.
    #[test]
    fn the_engine_artifact_is_embedded() {
        assert!(
            host_artifact().is_some(),
            "no engine artifact embedded. Run `npm run build:engine` (or `cargo build -p \
             apex-engine`) before building the client; build.rs reads its output."
        );
    }

    /// The digest is computed by `build.rs` without a dependency, and verification happens on
    /// the remote host with `sha256sum`. If the two disagree, every deployment fails
    /// verification against a host that is behaving perfectly — so the implementation is
    /// checked against the tool it must agree with, not merely against itself.
    #[test]
    fn the_embedded_digest_matches_what_sha256sum_computes() {
        let Some((bytes, digest)) = host_artifact() else {
            return; // covered by the test above
        };
        let dir = std::env::temp_dir().join(format!("apex-digest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("artifact");
        std::fs::write(&path, bytes).expect("write artifact");

        let out = std::process::Command::new("sha256sum")
            .arg(&path)
            .output()
            .expect("sha256sum must be installed; it is what the remote host verifies with");
        let theirs = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            digest, theirs,
            "the digest build.rs computed disagrees with sha256sum. Every deployment would \
             fail verification against a host that did nothing wrong."
        );
    }
}
