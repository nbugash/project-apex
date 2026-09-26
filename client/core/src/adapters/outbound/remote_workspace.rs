//! Outbound adapter: `WorkspaceProvider` over the JSON-RPC transport.
//!
//! Translates one provider call into **at most one** protocol request (R1). No fan-out, no retry,
//! no prefetch — which is what makes SC-002 measure what it claims to: if a provider call could
//! issue two requests, the count of listings would stop equalling the count of folders expanded.

use crate::application::ports::bulk_transfer::BulkTransfer;
use crate::application::ports::request_sender::RequestSender;
use crate::application::ports::transport::Request;
use crate::application::ports::workspace_provider::{
    ProviderError, ProviderResult, Refusal, RefusalReason, WatchOutcome, WorkspaceProvider,
};
use crate::domain::request::RequestOutcome;
use crate::domain::workspace::{
    ByteRange, DirPage, FileChunk, FsEntry, FsMeta, PageRequest, RelPath, Sha256, WorkspaceId,
};
use apex_protocol::wire::{self, codes};
use async_trait::async_trait;
use std::sync::Arc;

pub struct RemoteWorkspaceProvider {
    transport: Arc<dyn RequestSender>,
    /// Where content above the inline threshold comes from (A-BULK, A-BULKSIZE).
    bulk: Option<Arc<dyn BulkTransfer>>,
    /// The workspace's absolute root on the engine's host, needed to name a file to the bulk path.
    remote_base: String,
}

impl RemoteWorkspaceProvider {
    pub fn new(
        transport: Arc<dyn RequestSender>,
        bulk: Option<Arc<dyn BulkTransfer>>,
        remote_base: String,
    ) -> Self {
        Self {
            transport,
            bulk,
            remote_base,
        }
    }

    /// One request, one outcome. Errors are mapped from §4.4 codes to typed variants, so a caller
    /// never parses a message.
    async fn call<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &P,
    ) -> ProviderResult<R> {
        let body = serde_json::to_string(params)
            .map_err(|e| ProviderError::Transport(format!("encoding {method}: {e}")))?;
        // Refused here rather than discovered at the codec. §4.1's cap is enforced on encode, so
        // an oversized frame already fails safely -- but it fails as a transport error, and a
        // transport error tells the developer to try again. Retrying a file that is too large to
        // frame fails identically forever, so the one failure that must not be described as
        // temporary is exactly the one that would be.
        //
        // The budget is measured against the **encoded** params, never the caller's byte count.
        // A write carries its content as a JSON string, where a quote costs two bytes and a
        // control character costs six: any limit predicted from the raw length is wrong for the
        // files most likely to hit it. The headroom covers the envelope -- jsonrpc, id, method
        // and the `Content-Length` line -- which the codec adds after this point.
        const ENVELOPE_HEADROOM: usize = 4 * 1024;
        let budget = apex_protocol::framing::MAX_FRAME_BYTES - ENVELOPE_HEADROOM;
        if body.len() > budget {
            return Err(ProviderError::TooLarge {
                total_size: body.len() as u64,
            });
        }
        match self
            .transport
            .send(Request::interactive(method, body))
            .await
        {
            RequestOutcome::Answered(json) => {
                let value: serde_json::Value = serde_json::from_str(&json)
                    .map_err(|e| ProviderError::Transport(format!("reply to {method}: {e}")))?;
                let result = value.get("result").cloned().unwrap_or(value);
                serde_json::from_value(result).map_err(|e| {
                    // A malformed result is an error, never a panic and never a default value:
                    // what the engine sends is untrusted (Principle VI).
                    ProviderError::Transport(format!("result of {method}: {e}"))
                })
            }
            RequestOutcome::Failed { code, message } => Err(map_code(code, message)),
            RequestOutcome::TimedOut => Err(ProviderError::Transport("timed out".into())),
            RequestOutcome::Withdrawn => Err(ProviderError::Transport("withdrawn".into())),
            RequestOutcome::ConnectionLost => Err(ProviderError::Offline),
        }
    }
}

/// §4.4's codes, as the typed errors a caller branches on.
fn map_code(code: i32, message: String) -> ProviderError {
    match code {
        codes::WORKSPACE_NOT_REGISTERED => ProviderError::UnknownWorkspace,
        codes::WORKSPACE_GONE => ProviderError::WorkspaceGone,
        codes::PATH_REFUSED => ProviderError::Refused,
        codes::NOT_FOUND => ProviderError::NotFound,
        codes::WRITE_CONFLICT => ProviderError::WriteConflict,
        _ => ProviderError::Transport(format!("{code}: {message}")),
    }
}

fn decode_base64(s: &str) -> Option<Vec<u8>> {
    const INVALID: u8 = 255;
    let mut table = [INVALID; 256];
    for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .iter()
        .enumerate()
    {
        table[*c as usize] = i as u8;
    }
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|b| *b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut n: u32 = 0;
        for (i, b) in chunk.iter().enumerate() {
            let v = table[*b as usize];
            if v == INVALID {
                return None;
            }
            n |= (v as u32) << (18 - 6 * i);
        }
        let take = chunk.len() - 1;
        for i in 0..take {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[async_trait]
impl WorkspaceProvider for RemoteWorkspaceProvider {
    async fn watch(&self, ws: &WorkspaceId, paths: &[RelPath]) -> ProviderResult<WatchOutcome> {
        let params = wire::WatchParams {
            workspace_id: ws.clone(),
            paths: paths.iter().map(|p| p.as_str().to_string()).collect(),
        };
        let r: wire::WatchResult = self.call("workspace/watch", &params).await?;
        Ok(WatchOutcome {
            watching: r.watching,
            refused: r
                .refused
                .into_iter()
                .filter_map(from_wire_refusal)
                .collect(),
        })
    }

    async fn unwatch(&self, ws: &WorkspaceId, paths: &[RelPath]) -> ProviderResult<WatchOutcome> {
        let params = wire::WatchParams {
            workspace_id: ws.clone(),
            paths: paths.iter().map(|p| p.as_str().to_string()).collect(),
        };
        let r: wire::UnwatchResult = self.call("workspace/unwatch", &params).await?;
        Ok(WatchOutcome {
            watching: r.watching,
            refused: Vec::new(),
        })
    }

    async fn read_directory(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        page: PageRequest,
    ) -> ProviderResult<DirPage> {
        let params = wire::ReadDirectoryParams {
            workspace_id: ws.clone(),
            relative_path: path.as_str().to_string(),
            cursor: page.cursor.clone(),
            limit: Some(page.limit),
        };
        let r: wire::ReadDirectoryResult = self.call("workspace/readDirectory", &params).await?;
        Ok(DirPage {
            items: r
                .items
                .into_iter()
                .map(|e| FsEntry {
                    name: e.name,
                    kind: e.kind,
                    size: e.size,
                    modified: e.modified,
                })
                .collect(),
            next_cursor: r.next_cursor,
        })
    }

    async fn stat(&self, ws: &WorkspaceId, path: &RelPath) -> ProviderResult<FsMeta> {
        let params = wire::StatParams {
            workspace_id: ws.clone(),
            relative_path: path.as_str().to_string(),
        };
        let r: wire::StatResult = self.call("workspace/stat", &params).await?;
        Ok(FsMeta {
            kind: r.kind,
            size: r.size,
            modified: r.modified,
            sha256: r.sha256.as_deref().and_then(Sha256::parse),
        })
    }

    async fn read_file(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        // Above the threshold, content leaves the control channel entirely rather than being
        // chunked through it: one pipe is one queue, and a large response would serialise ahead
        // of every interactive request behind it (§4.6, A-BULK).
        // Only a caller that *named* a length above the cap is pre-routed. An unranged read asks
        // for the whole file without knowing its size, and treating that as "could be huge" sent
        // every whole-file read to the bulk path -- which nothing implements -- so reading a
        // ten-byte file failed. It went unnoticed because no test ever constructed this adapter
        // and the application never wired it; it surfaced the moment F006 needed a file open.
        if let Some(r) = range {
            if r.length > wire::MAX_INLINE_READ {
                return self.bulk_read(ws, path, range).await;
            }
        }

        let params = wire::ReadFileParams {
            workspace_id: ws.clone(),
            relative_path: path.as_str().to_string(),
            offset: range.map(|r| r.offset),
            length: range.map(|r| r.length),
        };
        match self
            .call::<_, wire::ReadFileResult>("workspace/readFile", &params)
            .await
        {
            Ok(r) => {
                let bytes = decode_base64(&r.content)
                    .ok_or_else(|| ProviderError::Transport("malformed base64 content".into()))?;
                let sha256 = Sha256::parse(&r.sha256)
                    .ok_or_else(|| ProviderError::Transport("malformed digest".into()))?;
                Ok(FileChunk {
                    range: ByteRange {
                        offset: range.map(|r| r.offset).unwrap_or(0),
                        length: bytes.len() as u64,
                    },
                    bytes,
                    total_size: r.total_size,
                    sha256,
                })
            }
            // The engine refuses rather than truncating, and the refusal is the signal to take
            // the bulk path.
            Err(ProviderError::Transport(ref m)) if m.contains("exceeds the inline read limit") => {
                self.bulk_read(ws, path, range).await
            }
            Err(e) => Err(e),
        }
    }

    /// Save, conditional on the base (FR-007, FR-008).
    ///
    /// The base is the whole method. Without it a save is an unconditional overwrite, and the
    /// developer whose colleague edited the file in between loses that colleague's work with no
    /// signal that anything happened. The engine does the comparing -- this side only has to
    /// carry the base honestly and keep the refusal legible when it comes back.
    async fn write_file(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        content: &[u8],
        base: &Sha256,
    ) -> ProviderResult<Sha256> {
        // §4.8 gives `writeFile` a plain `content` string and, unlike `readFile`, no `encoding`
        // field: the write path carries text and nothing else. Refused rather than converted,
        // because a lossy conversion replaces each invalid sequence with U+FFFD and saves *that*
        // -- the developer's file destroyed by the act of saving it. The same reasoning as
        // FR-006 on the way in, applied on the way out.
        let text = std::str::from_utf8(content).map_err(|_| {
            ProviderError::Transport(
                "content is not valid UTF-8, which writeFile cannot carry".into(),
            )
        })?;

        let params = wire::WriteFileParams {
            workspace_id: ws.clone(),
            relative_path: path.as_str().to_string(),
            content: text.to_string(),
            base_sha256: base.as_str().to_string(),
        };
        let r: wire::WriteFileResult = self.call("workspace/writeFile", &params).await?;

        // What the engine sends is untrusted (Principle VI). A malformed digest adopted as the
        // next base would never match anything, so the *following* save would be refused for a
        // conflict nobody caused -- reported, weeks later, as "it randomly stops saving".
        Sha256::parse(&r.sha256)
            .ok_or_else(|| ProviderError::Transport("malformed digest in writeFile result".into()))
    }
}

impl RemoteWorkspaceProvider {
    async fn bulk_read(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        let Some(bulk) = &self.bulk else {
            return Err(ProviderError::TooLarge { total_size: 0 });
        };
        // The bulk path returns bytes with **no integrity claim** — nothing about the transfer
        // authenticates the content — so the digest is taken from `stat` and the content checked
        // against it here.
        let meta = self.stat(ws, path).await?;
        let remote = format!(
            "{}{}",
            self.remote_base.trim_end_matches('/'),
            path.as_str()
        );
        let bytes = bulk
            .fetch(&remote, range)
            .await
            .map_err(|e| ProviderError::Transport(format!("bulk fetch: {e:?}")))?;

        let expected = meta
            .sha256
            .ok_or_else(|| ProviderError::Transport("no digest to verify a bulk read".into()))?;
        if range.is_none() && Sha256::of(&bytes) != expected {
            return Err(ProviderError::Transport(
                "bulk content does not match the engine's digest".into(),
            ));
        }
        Ok(FileChunk {
            range: ByteRange {
                offset: range.map(|r| r.offset).unwrap_or(0),
                length: bytes.len() as u64,
            },
            total_size: meta.size,
            bytes,
            sha256: expected,
        })
    }
}

/// A refusal whose path does not parse is dropped rather than trusted.
///
/// Every path off the wire is untrusted at this end regardless of what the engine checked
/// (FR-014, Principle VI). A refusal is the engine telling us about a path we sent, so a
/// malformed one means the two ends disagree about what was asked -- and acting on it would be
/// acting on the engine's word about our own request.
fn from_wire_refusal(r: apex_protocol::wire::WatchRefusal) -> Option<Refusal> {
    use apex_protocol::wire::RefusalReason as Wire;
    Some(Refusal {
        path: RelPath::parse(&r.path).ok()?,
        reason: match r.reason {
            Wire::Capacity => RefusalReason::Capacity,
            Wire::Excluded => RefusalReason::Excluded,
            Wire::NotFound => RefusalReason::NotFound,
            Wire::NotADirectory => RefusalReason::NotADirectory,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes_the_rfc_vectors_and_refuses_junk() {
        // The encoder lives in the engine and the decoder here; they never share code, so both
        // carry the vectors. A round trip alone would pass for two agreeing wrong implementations.
        for (encoded, plain) in [
            ("", &b""[..]),
            ("Zg==", b"f"),
            ("Zm8=", b"fo"),
            ("Zm9v", b"foo"),
            ("Zm9vYg==", b"foob"),
            ("Zm9vYmE=", b"fooba"),
            ("Zm9vYmFy", b"foobar"),
        ] {
            assert_eq!(decode_base64(encoded).as_deref(), Some(plain), "{encoded}");
        }
        assert_eq!(
            decode_base64("//79"),
            Some(vec![0xff, 0xfe, 0xfd]),
            "high bits survive"
        );
        assert_eq!(
            decode_base64("!!!!"),
            None,
            "junk is refused rather than silently mangled"
        );
    }

    #[test]
    fn wire_codes_map_to_the_variants_that_lead_to_different_responses() {
        assert_eq!(
            map_code(codes::WORKSPACE_NOT_REGISTERED, "x".into()),
            ProviderError::UnknownWorkspace
        );
        assert_eq!(
            map_code(codes::WORKSPACE_GONE, "x".into()),
            ProviderError::WorkspaceGone
        );
        assert_ne!(
            map_code(codes::WORKSPACE_GONE, "x".into()),
            map_code(codes::WORKSPACE_NOT_REGISTERED, "x".into()),
            "these lead to opposite client behaviour: one says re-register, the other says tell \
             the developer and stop projecting"
        );
        assert_eq!(
            map_code(codes::PATH_REFUSED, "x".into()),
            ProviderError::Refused
        );
        assert_eq!(
            map_code(codes::NOT_FOUND, "x".into()),
            ProviderError::NotFound
        );
    }
}
