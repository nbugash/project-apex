//! A `GitProvider` whose pages are scripted, so paging is testable without a connection.

use apex_protocol::wire::{GitDiffResult, GitStatusResult};
use apex_shell::application::ports::git_provider::GitProvider;
use apex_shell::application::ports::workspace_provider::{ProviderError, ProviderResult};
use apex_shell::domain::workspace::WorkspaceId;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct FakeGit {
    /// cursor -> what asking for it returns. An absent cursor is a refusal, which is what a
    /// superseded snapshot produces.
    pages: Mutex<HashMap<String, GitStatusResult>>,
    /// Every cursor asked for, in order. The evidence for "no request was issued".
    pub asked: Mutex<Vec<String>>,
    diff: Mutex<GitDiffResult>,
}

impl FakeGit {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn page(&self, cursor: &str, result: GitStatusResult) {
        self.pages
            .lock()
            .unwrap()
            .insert(cursor.to_string(), result);
    }

    pub fn set_diff(&self, diff: GitDiffResult) {
        *self.diff.lock().unwrap() = diff;
    }

    pub fn asked_for(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl GitProvider for FakeGit {
    async fn status(
        &self,
        _workspace: &WorkspaceId,
        cursor: Option<&str>,
    ) -> ProviderResult<GitStatusResult> {
        let key = cursor.unwrap_or("").to_string();
        self.asked.lock().unwrap().push(key.clone());
        self.pages
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or_else(|| ProviderError::Transport(format!("-32602: unknown cursor {key}")))
    }

    async fn file_diff(
        &self,
        _workspace: &WorkspaceId,
        _relative_path: &str,
    ) -> ProviderResult<GitDiffResult> {
        Ok(self.diff.lock().unwrap().clone())
    }
}
