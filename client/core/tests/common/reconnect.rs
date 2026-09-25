//! The reattachment double, shared by the three suites that drive the sequence.
//!
//! One copy, because three would drift: a recorder that answered differently in one file from
//! another would make two of those files agree with the recorder rather than with the engine.

#![allow(dead_code)] // each suite uses a different subset

use apex_protocol::wire::{SignalName, TaskId};
use apex_shell::application::ports::task_provider::{
    AttachResult, Pid, StartRequest, TaskProvider, TaskSummary, TerminateSignal,
};
use apex_shell::application::ports::workspace_provider::{
    ProviderError, ProviderResult, WatchOutcome, WorkspaceProvider,
};
use apex_shell::domain::workspace::{
    ByteRange, DirPage, FileChunk, FsMeta, PageRequest, RelPath, WorkspaceId,
};
use async_trait::async_trait;
use std::sync::Mutex;

/// Records what it was asked to do, and answers however a case needs.
#[derive(Default)]
pub struct Recorder {
    pub calls: Mutex<Vec<String>>,
    pub listed: Mutex<Vec<TaskSummary>>,
    pub attaches: Mutex<std::collections::BTreeMap<String, ProviderResult<AttachResult>>>,
}

impl Recorder {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
    pub fn answer(&self, task: &str, with: ProviderResult<AttachResult>) {
        self.attaches
            .lock()
            .expect("attaches")
            .insert(task.to_string(), with);
    }
}

pub fn running(retained: u64) -> ProviderResult<AttachResult> {
    Ok(AttachResult {
        pid: Pid(1),
        running: true,
        retained,
        exit_code: None,
        signal: None,
    })
}

pub fn finished(code: i32) -> ProviderResult<AttachResult> {
    Ok(AttachResult {
        pid: Pid(1),
        running: false,
        retained: 0,
        exit_code: Some(code),
        signal: None,
    })
}

#[async_trait]
impl WorkspaceProvider for Recorder {
    // The three §6.1 requires. Everything else has a default that refuses, and refusing is the
    // right answer here: a reattachment reads no files.
    async fn read_directory(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _page: PageRequest,
    ) -> ProviderResult<DirPage> {
        Err(ProviderError::Offline)
    }
    async fn stat(&self, _ws: &WorkspaceId, _path: &RelPath) -> ProviderResult<FsMeta> {
        Err(ProviderError::Offline)
    }
    async fn read_file(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        Err(ProviderError::Offline)
    }
    async fn watch(&self, _ws: &WorkspaceId, _paths: &[RelPath]) -> ProviderResult<WatchOutcome> {
        self.calls.lock().expect("calls").push("watch".into());
        Ok(WatchOutcome {
            watching: 1,
            refused: Vec::new(),
        })
    }
}

#[async_trait]
impl TaskProvider for Recorder {
    async fn start(&self, _request: &StartRequest) -> ProviderResult<Pid> {
        Err(ProviderError::Offline)
    }
    async fn attach(&self, _ws: &WorkspaceId, task: &TaskId) -> ProviderResult<AttachResult> {
        self.calls
            .lock()
            .expect("calls")
            .push(format!("attach:{}", task.0));
        self.attaches
            .lock()
            .expect("attaches")
            .get(&task.0)
            .cloned()
            .unwrap_or(Err(ProviderError::TaskNotFound))
    }
    async fn list(&self, _ws: Option<&WorkspaceId>) -> ProviderResult<Vec<TaskSummary>> {
        self.calls.lock().expect("calls").push("list".into());
        Ok(self.listed.lock().expect("listed").clone())
    }
    async fn write_stdin(&self, _task: &TaskId, _data: &[u8]) -> ProviderResult<()> {
        Ok(())
    }
    async fn resize(&self, task: &TaskId, _cols: u16, _rows: u16) -> ProviderResult<()> {
        self.calls
            .lock()
            .expect("calls")
            .push(format!("resize:{}", task.0));
        Ok(())
    }
    async fn terminate(&self, _task: &TaskId, _signal: TerminateSignal) -> ProviderResult<()> {
        Ok(())
    }
    async fn close_workspace(&self, _ws: &WorkspaceId) -> ProviderResult<()> {
        Ok(())
    }
}

impl Recorder {
    /// What `execution/list` will return: one running task with this id.
    pub fn list_returns(&self, task: &str) {
        self.listed.lock().expect("listed").push(TaskSummary {
            task_id: TaskId(task.into()),
            workspace_id: apex_protocol::wire::WorkspaceId("ws1".into()),
            command: vec!["cargo".into()],
            pty: true,
            pid: Pid(7),
            running: true,
            exit_code: None,
            signal: None,
        });
    }
}

/// An attach result for a task killed by a signal.
pub fn signalled(name: &str) -> ProviderResult<AttachResult> {
    Ok(AttachResult {
        pid: Pid(1),
        running: false,
        retained: 0,
        exit_code: None,
        signal: Some(SignalName(name.into())),
    })
}

/// The shape §4.8 says cannot happen: both fields present. A client that picked whichever was
/// there would read a signalled death as a clean exit.
pub fn both_fields() -> ProviderResult<AttachResult> {
    Ok(AttachResult {
        pid: Pid(1),
        running: false,
        retained: 0,
        exit_code: Some(0),
        signal: Some(SignalName("SIGKILL".into())),
    })
}
