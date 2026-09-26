//! The local task provider: refuses everything, and says whose job it is.
//!
//! §6.4 has a local provider and F015 `local-mode` owns it. Running a task on the developer's
//! own machine is not a smaller version of running one on the instance -- there is no transport
//! to carry the output, no `execution/*` frame to send, and no reason for either, because the
//! client would be talking to a process it started itself.
//!
//! A file rather than a silence, following A-WATCHLOCAL's precedent. An absent implementation is
//! indistinguishable from an oversight; a present one that names its owner turns a log line into
//! a schedule (FR-004). research.md's *Local mode is F015's* has the reasoning.

use async_trait::async_trait;

use crate::application::ports::task_provider::{
    AttachResult, Pid, StartRequest, TaskId, TaskProvider, TaskSummary, TerminateSignal,
    LOCAL_OWNER,
};
use crate::application::ports::workspace_provider::{ProviderError, ProviderResult};
use crate::domain::workspace::WorkspaceId;

/// Every method refuses with the same owner. There is deliberately no partial implementation:
/// a provider that started tasks but could not stream their output would be worse than one that
/// refuses, because the failure would arrive after the developer had started a build.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalTasks;

fn unsupported<T>() -> ProviderResult<T> {
    Err(ProviderError::Unsupported { owner: LOCAL_OWNER })
}

#[async_trait]
impl TaskProvider for LocalTasks {
    async fn start(&self, _request: &StartRequest) -> ProviderResult<Pid> {
        unsupported()
    }

    async fn attach(&self, _ws: &WorkspaceId, _task: &TaskId) -> ProviderResult<AttachResult> {
        unsupported()
    }

    async fn list(&self, _ws: Option<&WorkspaceId>) -> ProviderResult<Vec<TaskSummary>> {
        unsupported()
    }

    async fn write_stdin(&self, _task: &TaskId, _data: &[u8]) -> ProviderResult<()> {
        unsupported()
    }

    async fn resize(&self, _task: &TaskId, _cols: u16, _rows: u16) -> ProviderResult<()> {
        unsupported()
    }

    async fn terminate(&self, _task: &TaskId, _signal: TerminateSignal) -> ProviderResult<()> {
        unsupported()
    }

    async fn close_workspace(&self, _ws: &WorkspaceId) -> ProviderResult<()> {
        unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::workspace_provider::Owner;

    #[tokio::test]
    async fn every_method_names_the_feature_that_owns_it() {
        let p = LocalTasks;
        let ws = WorkspaceId("ws-1".into());
        let task = TaskId("build".into());

        // All seven. A partial refusal would leave a caller unable to tell "not yet" from
        // "went wrong", which is the distinction FR-004 exists to preserve.
        let errors = vec![
            p.attach(&ws, &task).await.err(),
            p.list(None).await.err(),
            p.write_stdin(&task, b"x").await.err(),
            p.resize(&task, 80, 24).await.err(),
            p.terminate(&task, TerminateSignal::Term).await.err(),
            p.close_workspace(&ws).await.err(),
        ];
        for e in errors {
            assert_eq!(
                e,
                Some(ProviderError::Unsupported {
                    owner: Owner::F015LocalMode
                })
            );
        }
    }

    #[test]
    fn the_owner_reads_as_a_schedule() {
        // A log line saying "not implemented" is a bug report; one naming the feature is a
        // schedule.
        assert_eq!(
            ProviderError::Unsupported {
                owner: Owner::F015LocalMode
            }
            .to_string(),
            "not implemented here; F015 local-mode owns it"
        );
    }
}
