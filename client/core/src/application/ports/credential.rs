//! Asking the interface layer for a passphrase.
//!
//! Inbound in direction: the core calls out to the interface, which renders the prompt in
//! the application's own window. An IDE that drops the user into a terminal to authenticate
//! has failed at the thing it exists to do.

use crate::domain::request::Secret;

#[derive(Debug, Clone)]
pub struct PromptContext {
    /// OpenSSH's own prompt text, passed through rather than reworded — it names the key
    /// file, which is what lets the user tell which credential is being asked for.
    pub prompt: String,
    pub host: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("the user dismissed the prompt")]
    Cancelled,
    #[error("no interface is available to prompt with")]
    Unavailable,
}

#[allow(async_fn_in_trait)]
pub trait CredentialPrompt: Send + Sync {
    /// Called only after an `AuthenticationFailed` classification (FR-006).
    async fn passphrase(&self, ctx: PromptContext) -> Result<Secret, PromptError>;
}
