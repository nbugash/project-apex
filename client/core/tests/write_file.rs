//! `RemoteWorkspaceProvider::write_file`, and what each engine answer becomes.
//!
//! Against a scripted transport rather than an engine, because what is under test is the
//! **mapping**: every §4.4 code the write path can return has to arrive at the caller as a
//! distinct thing, and driving a real engine to produce all six would be harder and prove less.
//!
//! The distinction that matters most is conflict against unreachable. One means a colleague
//! edited the file and the developer should look at what they did; the other means the link
//! dropped and they should try again. Collapsing them would make the next action a guess.

use apex_protocol::wire::codes;
use apex_shell::adapters::outbound::remote_workspace::RemoteWorkspaceProvider;
use apex_shell::application::ports::request_sender::RequestSender;
use apex_shell::application::ports::transport::Request;
use apex_shell::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
use apex_shell::domain::request::RequestOutcome;
use apex_shell::domain::workspace::{RelPath, Sha256, WorkspaceId};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// Answers with whatever it was handed, and remembers what it was asked.
struct Scripted {
    answer: Mutex<Option<RequestOutcome>>,
    seen: Mutex<Vec<Request>>,
}

impl Scripted {
    fn new(answer: RequestOutcome) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(Some(answer)),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn failing(code: i32) -> Arc<Self> {
        Self::new(RequestOutcome::Failed {
            code,
            message: "engine said so".into(),
        })
    }

    /// A success carrying whatever the engine claims the resulting digest is.
    fn answering(sha256: &str) -> Arc<Self> {
        Self::new(RequestOutcome::Answered(format!(
            r#"{{"result":{{"sha256":"{sha256}"}}}}"#
        )))
    }
}

#[async_trait]
impl RequestSender for Scripted {
    async fn send(&self, request: Request) -> RequestOutcome {
        self.seen.lock().expect("seen").push(request);
        self.answer
            .lock()
            .expect("answer")
            .take()
            .unwrap_or(RequestOutcome::ConnectionLost)
    }

    async fn notify(&self, request: Request) {
        self.seen.lock().expect("seen").push(request);
    }
}

fn provider(sender: Arc<Scripted>) -> RemoteWorkspaceProvider {
    RemoteWorkspaceProvider::new(sender, None, "/remote".into())
}

/// The digest of the content the file held before this save. Built by hashing rather than typed,
/// because `Sha256` has no constructor that takes a literal -- which is the point of the type.
fn base() -> Sha256 {
    Sha256::of(b"what was there before")
}

async fn write_bytes(sender: Arc<Scripted>, content: &[u8]) -> Result<Sha256, ProviderError> {
    provider(sender)
        .write_file(
            &WorkspaceId("w1".into()),
            &RelPath::parse("src/main.rs").expect("path"),
            content,
            &base(),
        )
        .await
}

async fn write(sender: Arc<Scripted>) -> Result<Sha256, ProviderError> {
    write_bytes(sender, b"fn main() {}").await
}

#[tokio::test]
async fn a_success_yields_the_hash_the_engine_returned() {
    let written = Sha256::of(b"fn main() {}");
    let got = write(Scripted::answering(written.as_str()))
        .await
        .expect("should succeed");
    assert_eq!(got, written);
}

#[tokio::test]
async fn a_conflict_is_its_own_error_and_not_a_transport_failure() {
    // The whole reason `WriteConflict` is a variant. A caller that had to parse a message to
    // find this would get it wrong the first time the message was reworded.
    let err = write(Scripted::failing(codes::WRITE_CONFLICT))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::WriteConflict), "got {err:?}");
}

#[tokio::test]
async fn a_lost_connection_is_offline_and_not_a_conflict() {
    // The pair that must never collapse: one means a colleague edited the file, the other means
    // the link dropped, and the developer's next action differs completely.
    let err = write(Scripted::new(RequestOutcome::ConnectionLost))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Offline), "got {err:?}");
}

#[tokio::test]
async fn a_refused_path_is_refused_and_not_not_found() {
    // -32002 and -32003 were transposed in this feature's own contract until the constants were
    // read. Telling a developer their file is missing when the engine refused the path sends
    // them looking for the wrong thing.
    let err = write(Scripted::failing(codes::PATH_REFUSED))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Refused), "got {err:?}");
}

#[tokio::test]
async fn a_missing_file_is_not_found_and_not_a_conflict() {
    let err = write(Scripted::failing(codes::NOT_FOUND)).await.unwrap_err();
    assert!(matches!(err, ProviderError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn an_unregistered_workspace_says_to_re_register() {
    let err = write(Scripted::failing(codes::WORKSPACE_NOT_REGISTERED))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::UnknownWorkspace), "got {err:?}");
}

#[tokio::test]
async fn a_malformed_digest_is_an_error_rather_than_a_silent_miss() {
    // `Sha256::parse` refuses anything that is not 64 lowercase hex, and the adapter must turn
    // that refusal into an error. Adopting a malformed base would make the *next* save fail with
    // a conflict nobody caused, which is a bug reported as "it randomly stops saving".
    let err = write(Scripted::answering("not-a-digest")).await.unwrap_err();
    assert!(matches!(err, ProviderError::Transport(_)), "got {err:?}");
}

#[tokio::test]
async fn content_that_is_not_text_is_refused_rather_than_mangled() {
    // §4.8 gives `writeFile` a plain `content` string and, unlike `readFile`, no `encoding`
    // field: the write path carries text and nothing else. The failure this guards is a lossy
    // conversion, which would replace each invalid sequence with U+FFFD and save *that* -- the
    // developer's file destroyed by the act of saving it.
    let sender = Scripted::answering(Sha256::of(b"x").as_str());
    let err = write_bytes(sender.clone(), &[0xff, 0xfe, 0x00]).await.unwrap_err();
    assert!(matches!(err, ProviderError::Transport(_)), "got {err:?}");
    assert!(
        sender.seen.lock().expect("seen").is_empty(),
        "nothing may be sent for content the wire cannot carry"
    );
}

#[tokio::test]
async fn the_request_carries_the_base_and_the_content() {
    // Without the base the engine cannot refuse anything, so a write that dropped it would turn
    // every save into an unconditional overwrite while still looking like it worked.
    let sender = Scripted::answering(Sha256::of(b"fn main() {}").as_str());
    let _ = write(sender.clone()).await;

    let seen = sender.seen.lock().expect("seen");
    assert_eq!(seen.len(), 1, "one provider call is one request (R1)");
    let request = seen.first().expect("one request");
    assert_eq!(request.method, "workspace/writeFile");
    // snake_case on the wire, whatever §4.8's tables print for readability (A-WIRECASE).
    assert!(request.params.contains("base_sha256"), "{}", request.params);
    assert!(
        request.params.contains(base().as_str()),
        "the base digest must be the one the caller passed: {}",
        request.params
    );
    assert!(request.params.contains("fn main"), "{}", request.params);
}

#[tokio::test]
async fn a_write_too_large_to_frame_is_refused_before_it_is_sent() {
    // §4.1's cap is enforced on encode, so an oversized frame already fails safely. It fails as
    // a *transport* error though, and a transport error means "try again" -- advice that is
    // wrong forever for a file that cannot be framed. Refusing here is what makes the one
    // permanent failure stop describing itself as temporary.
    let sender = Scripted::answering(Sha256::of(b"x").as_str());
    let huge = vec![b'x'; apex_protocol::framing::MAX_FRAME_BYTES];

    let err = write_bytes(sender.clone(), &huge).await.unwrap_err();

    assert!(matches!(err, ProviderError::TooLarge { .. }), "got {err:?}");
    assert!(
        sender.seen.lock().expect("seen").is_empty(),
        "a frame that cannot be encoded must not be handed to the transport"
    );
}

#[tokio::test]
async fn the_budget_is_measured_on_the_encoded_frame_and_not_the_byte_count() {
    // A quote costs two bytes as JSON and a control character costs six, so a limit predicted
    // from the caller's byte count is wrong for exactly the files most likely to reach it. This
    // content is comfortably under the cap raw and over it once escaped.
    let sender = Scripted::answering(Sha256::of(b"x").as_str());
    let quotes = vec![b'"'; apex_protocol::framing::MAX_FRAME_BYTES * 2 / 3];
    assert!(
        quotes.len() < apex_protocol::framing::MAX_FRAME_BYTES,
        "the fixture must be under the cap before escaping, or it proves nothing"
    );

    let err = write_bytes(sender, &quotes).await.unwrap_err();

    assert!(matches!(err, ProviderError::TooLarge { .. }), "got {err:?}");
}
