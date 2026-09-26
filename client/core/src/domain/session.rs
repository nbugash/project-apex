//! Session state. See specs/001-app-shell/data-model.md and
//! specs/001-app-shell/contracts/session-state.schema.json.

use super::geometry::{DisplayBounds, WindowGeometry};
use super::layout::Layout;
use super::rail::{RailCatalogue, ToolWindowState};
use serde::{Deserialize, Serialize};

/// Raised to 2 by the tool window fields F018 added, and to 3 by F010's task identities.
///
/// A store written at an **older** version still loads: every field added since carries
/// `serde(default)`, so a version 1 file parses and gets the defaults rather than failing, which
/// is what keeps an existing user's geometry, layout and tabs through an upgrade. A store written
/// at a **newer** version is refused, because this image cannot know what it would be discarding.
pub const SCHEMA_VERSION: u32 = 4;
const MAX_NAME: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub String);

impl DocumentId {
    /// Opaque and not derived from a path: a document whose path changes keeps its tab, its
    /// position and its focus. Same reasoning as the cache identity correction in A-B5.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LocationType {
    Remote,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReference {
    /// The identity the engine and the cache both key on, added at schema version 4.
    ///
    /// Without it the interface could open a workspace and then had no way to name it again:
    /// the tree, which asks for a listing *by workspace id*, was constructed with a literal
    /// `"e2e"` and could only ever show what the debug seeding command had put in the cache.
    /// Restoring one after a restart was impossible for the same reason.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub location_type: LocationType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenDocumentReference {
    pub id: DocumentId,
    pub display_name: String,
    pub order: u32,
    /// Which file the tab is of, added at schema version 4 (FR-021).
    ///
    /// Separate from `display_name` because they answer different questions. The name is for a
    /// person reading a tab strip, where a full path would be unreadable; the path is what an
    /// editor reads and what a restored tab needs in order to fetch anything at all. A tab that
    /// remembered only its name could be restored as a label with nothing behind it, which is
    /// what FR-022 calls presenting an empty buffer.
    ///
    /// `serde(default)` leaves a version 3 tab with an empty path. That tab is restored as a
    /// label whose content cannot be fetched -- which is correct: the previous release never
    /// recorded which file it was, and inventing one from the display name would open whichever
    /// file happened to share the name.
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    InvalidDisplayName,
    UnknownDocument,
    OrderOutOfRange,
}

/// The on-disk shape. Carries `schema_version`; `SessionSnapshot` does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedSession {
    pub schema_version: u32,
    pub workspace: Option<WorkspaceReference>,
    pub window: WindowGeometry,
    pub layout: Layout,
    pub documents: Vec<OpenDocumentReference>,
    pub focused_document_id: Option<DocumentId>,
    /// Added at schema version 2. `serde(default)` is the migration: a version 1 file has
    /// no such field and gets the default rather than failing to parse, which is what
    /// keeps an existing user's geometry, layout and tabs through the upgrade.
    #[serde(default)]
    pub tool_window: ToolWindowState,
    /// Added at schema version 3 (A-STATE2, FR-031d). The same migration for the same reason.
    ///
    /// **The identities and nothing else.** A task outlives the connection that started it, and
    /// `execution/attach` reaches one by an identity the client must already know -- so a client
    /// that restarts needs its identities to have survived the restart, or reattachment works
    /// only for a client that never closed.
    ///
    /// No command, and no output. Storing a command would put a credential passed in argv on
    /// disk, which FR-005a's accepted boundary does not extend to; storing output would make the
    /// store grow without bound for a client that never returns. The identity is the smallest
    /// thing that restores reachability, and `execution/list` covers the client that has lost
    /// even that.
    #[serde(default)]
    pub tasks: Vec<PersistedTask>,
    /// Added at schema version 4 (FR-007b). The same `serde(default)` migration for the same
    /// reason A-STATE2 established.
    ///
    /// **`false` when unset, and that is not merely a convenient default.** Autosave writes the
    /// developer's file without them asking, and a profile that has never expressed a preference
    /// has not agreed to that. `bool::default()` happening to be `false` is the right value for
    /// the right reason, which is worth saying because the next person to add a field here will
    /// reach for the default without asking whether it is the safe one.
    #[serde(default)]
    pub autosave: bool,
}

/// One task this client started, as the store remembers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedTask {
    pub task_id: String,
    /// Which workspace owns it. `execution/attach` takes both, and a task whose workspace the
    /// client has forgotten cannot be attached to -- it would have to enumerate to find it again,
    /// which is the recovery path rather than the ordinary one.
    pub workspace_id: String,
}

impl Default for PersistedSession {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            workspace: None,
            window: WindowGeometry::default(),
            layout: Layout::default(),
            documents: Vec::new(),
            focused_document_id: None,
            tool_window: ToolWindowState::default(),
            tasks: Vec::new(),
            autosave: false,
        }
    }
}

/// What crosses the bridge. Omits `schema_version`: a storage format version is meaningless
/// to the interface layer, and exposing it would invite the webview to branch on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub workspace: Option<WorkspaceReference>,
    pub window: WindowGeometry,
    pub layout: Layout,
    pub documents: Vec<OpenDocumentReference>,
    pub focused_document_id: Option<DocumentId>,
    /// FR-011. Without this the interface could persist tool window state but never read it
    /// back, so a restart would silently reset the panel every time.
    pub tool_window: ToolWindowState,
    /// The identities a restarted client reattaches to (A-STATE2, FR-031d). Crossing the bridge
    /// because the panel is what reattaches, and it cannot ask for what it was never told.
    pub tasks: Vec<PersistedTask>,
    /// Whether to save without being asked (FR-007b). Off for a profile that never set it.
    pub autosave: bool,
}

impl From<&PersistedSession> for SessionSnapshot {
    fn from(p: &PersistedSession) -> Self {
        Self {
            workspace: p.workspace.clone(),
            window: p.window,
            layout: p.layout,
            documents: p.documents.clone(),
            focused_document_id: p.focused_document_id.clone(),
            tool_window: p.tool_window.clone(),
            tasks: p.tasks.clone(),
            autosave: p.autosave,
        }
    }
}

impl PersistedSession {
    /// Invariants that JSON Schema cannot express. A file failing any of these is discarded
    /// in full rather than partially recovered — a half-restored layout is harder to reason
    /// about than a default one.
    pub fn is_coherent(&self) -> bool {
        // A file from the FUTURE is discarded: this build cannot interpret a shape it does
        // not know, and guessing risks corrupting it on the next write. An OLDER file is
        // migrated forward, not discarded — the previous rule rejected anything that was
        // not exactly current, which was right with one version and became data loss the
        // moment a second existed.
        if self.schema_version > SCHEMA_VERSION {
            return false;
        }
        let mut ids: Vec<&DocumentId> = self.documents.iter().map(|d| &d.id).collect();
        let before = ids.len();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        ids.dedup();
        if ids.len() != before {
            return false;
        }
        let mut orders: Vec<u32> = self.documents.iter().map(|d| d.order).collect();
        orders.sort_unstable();
        if orders != (0..self.documents.len() as u32).collect::<Vec<_>>() {
            return false;
        }
        match &self.focused_document_id {
            None => self.documents.is_empty(),
            Some(id) => self.documents.iter().any(|d| &d.id == id),
        }
    }

    /// Repairs applied on load: clamping, and geometry recovery onto an attached display.
    pub fn repaired(mut self, displays: &[DisplayBounds]) -> Self {
        self.layout = self.layout.clamped();
        self.window = self.window.constrained_to(displays);

        // Fold the retired `navigation` region into the tool window that replaced it. Done
        // before the tool window's own repair so the folded width still gets clamped to the
        // panel's minimum, and only when the legacy field is present — otherwise a current
        // file would have its panel reset to a default on every load.
        if let Some(nav) = self.layout.legacy_navigation.take() {
            self.tool_window.width = nav.extent;
            self.tool_window.collapsed = !nav.visible;
        }

        self.tool_window = self.tool_window.repaired(&RailCatalogue::default());
        // Rewritten at the current version, so the migration happens once.
        self.schema_version = SCHEMA_VERSION;
        self
    }

    /// Open a tab for a file.
    ///
    /// Both the name and the path, because a tab that knows only what to call itself cannot be
    /// restored into anything (FR-021). A file already open is focused rather than opened twice:
    /// two tabs of one file would be two buffers, two bases, and a save through one silently
    /// reverting the other (FR-023).
    pub fn open_document(
        &mut self,
        display_name: &str,
        path: &str,
    ) -> Result<DocumentId, SessionError> {
        let trimmed = display_name.trim();
        if trimmed.is_empty() || display_name.chars().count() > MAX_NAME {
            return Err(SessionError::InvalidDisplayName);
        }
        if !path.is_empty() {
            if let Some(open) = self.documents.iter().find(|d| d.path == path) {
                let id = open.id.clone();
                self.focused_document_id = Some(id.clone());
                return Ok(id);
            }
        }
        let id = DocumentId::new();
        self.documents.push(OpenDocumentReference {
            id: id.clone(),
            display_name: display_name.to_string(),
            order: self.documents.len() as u32,
            path: path.to_string(),
        });
        self.focused_document_id = Some(id.clone());
        Ok(id)
    }

    /// Record which workspace is open, so the interface can name it again after a restart.
    pub fn set_workspace(&mut self, id: &str, name: &str, location_type: LocationType) {
        self.workspace = Some(WorkspaceReference {
            id: id.to_string(),
            name: name.to_string(),
            location_type,
        });
    }

    pub fn close_document(&mut self, id: &DocumentId) -> Result<(), SessionError> {
        let idx = self
            .documents
            .iter()
            .position(|d| &d.id == id)
            .ok_or(SessionError::UnknownDocument)?;
        self.documents.remove(idx);
        self.repack_orders();
        if self.focused_document_id.as_ref() == Some(id) {
            // Focus moves to the neighbour that took the closed document's place, or the
            // one before it when the last tab closed.
            self.focused_document_id = self
                .documents
                .get(idx)
                .or_else(|| idx.checked_sub(1).and_then(|i| self.documents.get(i)))
                .map(|d| d.id.clone());
        }
        Ok(())
    }

    pub fn reorder_document(&mut self, id: &DocumentId, to: u32) -> Result<(), SessionError> {
        if to as usize >= self.documents.len() {
            return Err(SessionError::OrderOutOfRange);
        }
        let from = self
            .documents
            .iter()
            .position(|d| &d.id == id)
            .ok_or(SessionError::UnknownDocument)?;
        let doc = self.documents.remove(from);
        self.documents.insert(to as usize, doc);
        self.repack_orders();
        Ok(())
    }

    pub fn focus_document(&mut self, id: &DocumentId) -> Result<(), SessionError> {
        if !self.documents.iter().any(|d| &d.id == id) {
            return Err(SessionError::UnknownDocument);
        }
        self.focused_document_id = Some(id.clone());
        Ok(())
    }

    fn repack_orders(&mut self) {
        for (i, d) in self.documents.iter_mut().enumerate() {
            d.order = i as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_docs(n: usize) -> (PersistedSession, Vec<DocumentId>) {
        let mut s = PersistedSession::default();
        let ids = (0..n)
            .map(|i| s.open_document(&format!("doc{i}"), &format!("/doc{i}")).unwrap())
            .collect();
        (s, ids)
    }

    #[test]
    fn opening_appends_and_focuses() {
        let (s, ids) = with_docs(3);
        assert_eq!(s.documents.len(), 3);
        assert_eq!(s.focused_document_id.as_ref(), ids.last());
        assert!(s.is_coherent());
    }

    #[test]
    fn empty_or_overlong_names_are_rejected() {
        let mut s = PersistedSession::default();
        assert_eq!(
            s.open_document("   ", "/x"),
            Err(SessionError::InvalidDisplayName)
        );
        assert_eq!(
            s.open_document(&"x".repeat(256), "/x"),
            Err(SessionError::InvalidDisplayName)
        );
    }

    #[test]
    fn closing_repacks_orders_contiguously() {
        let (mut s, ids) = with_docs(4);
        s.close_document(&ids[1]).unwrap();
        assert_eq!(
            s.documents.iter().map(|d| d.order).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(s.is_coherent());
    }

    #[test]
    fn closing_the_focused_document_moves_focus_to_its_neighbour() {
        let (mut s, ids) = with_docs(3);
        s.focus_document(&ids[1]).unwrap();
        s.close_document(&ids[1]).unwrap();
        assert_eq!(s.focused_document_id.as_ref(), Some(&ids[2]));
    }

    #[test]
    fn closing_the_last_remaining_document_clears_focus() {
        let (mut s, ids) = with_docs(1);
        s.close_document(&ids[0]).unwrap();
        assert!(s.focused_document_id.is_none());
        assert!(s.is_coherent(), "focus is null iff documents is empty");
    }

    #[test]
    fn reordering_keeps_orders_contiguous() {
        let (mut s, ids) = with_docs(4);
        s.reorder_document(&ids[3], 0).unwrap();
        assert_eq!(s.documents[0].id, ids[3]);
        assert_eq!(
            s.documents.iter().map(|d| d.order).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn out_of_range_and_unknown_operations_are_rejected() {
        let (mut s, ids) = with_docs(2);
        assert_eq!(
            s.reorder_document(&ids[0], 9),
            Err(SessionError::OrderOutOfRange)
        );
        let ghost = DocumentId::new();
        assert_eq!(s.close_document(&ghost), Err(SessionError::UnknownDocument));
        assert_eq!(s.focus_document(&ghost), Err(SessionError::UnknownDocument));
    }

    #[test]
    fn incoherent_states_are_detected() {
        let (mut s, _) = with_docs(2);
        s.focused_document_id = Some(DocumentId::new());
        assert!(!s.is_coherent(), "dangling focus reference");

        let (mut s, _) = with_docs(2);
        s.documents[1].order = 5;
        assert!(!s.is_coherent(), "non-contiguous order");

        let (mut s, _) = with_docs(2);
        s.schema_version = 99;
        assert!(!s.is_coherent(), "future schema version");
    }

    #[test]
    fn a_version_one_file_migrates_forward_keeping_everything_else() {
        // The exact shape a previous release wrote: no tool_window field at all.
        let v1 = r#"{
            "schema_version": 1,
            "workspace": null,
            "window": {"x": 240, "y": 160, "width": 1100, "height": 760, "maximized": false},
            "layout": {
                "navigation": {"visible": true, "extent": 300},
                "output": {"visible": false, "extent": 220},
                "document_area": {"visible": true, "extent": 0}
            },
            "documents": [{"id": "d1", "display_name": "main.rs", "order": 0}],
            "focused_document_id": "d1"
        }"#;

        let parsed: PersistedSession =
            serde_json::from_str(v1).expect("a version 1 file must still parse");
        assert!(
            parsed.is_coherent(),
            "an older version is valid, not incoherent"
        );

        // A real display must be supplied: with none attached, geometry correctly falls
        // back to the default, which would mask what this test is checking.
        let screen = [DisplayBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        }];
        let migrated = parsed.repaired(&screen);
        assert_eq!(
            migrated.schema_version, SCHEMA_VERSION,
            "rewritten at the current version"
        );
        // The whole point: nothing the user had is lost.
        assert_eq!(migrated.window.width, 1100);
        // The v1 file's `navigation` region is the tool window now: its width must survive
        // the rename, because to the user it is the same panel they sized.
        assert_eq!(migrated.tool_window.width, 300);
        assert!(
            migrated.layout.legacy_navigation.is_none(),
            "folded, not kept"
        );
        assert!(!migrated.layout.output.visible);
        assert_eq!(migrated.documents.len(), 1);
        assert_eq!(migrated.focused_document_id, Some(DocumentId("d1".into())));
        // A file written before the tool window existed gains it already repaired: the
        // panel opens on the first available destination rather than on nothing.
        assert_eq!(
            migrated.tool_window.active_destination_id,
            RailCatalogue::default()
                .first_available()
                .map(|d| d.id.clone())
        );
        assert!(!migrated.tool_window.collapsed, "the v1 panel was visible");
    }

    /// The fold must be conditional. An unconditional one would overwrite the tool window
    /// with a default-constructed legacy region on every load, silently resetting the
    /// panel for everyone whose file no longer has that field — which is everyone, one
    /// save after upgrading.
    #[test]
    fn a_file_without_the_legacy_region_keeps_its_tool_window() {
        let mut s = PersistedSession::default();
        s.tool_window.width = 420;
        s.tool_window.collapsed = true;
        assert!(s.layout.legacy_navigation.is_none());

        let repaired = s.repaired(&[]);
        assert_eq!(repaired.tool_window.width, 420);
        assert!(repaired.tool_window.collapsed);
    }

    #[test]
    fn the_current_version_loads_unchanged() {
        let mut s = PersistedSession::default();
        s.open_document("a.rs", "/a.rs").unwrap();
        let round_tripped: PersistedSession =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(round_tripped, s);
        assert!(round_tripped.is_coherent());
    }

    #[test]
    fn a_future_version_is_still_discarded() {
        let s = PersistedSession {
            schema_version: SCHEMA_VERSION + 1,
            ..PersistedSession::default()
        };
        assert!(
            !s.is_coherent(),
            "a build cannot interpret a shape it does not know; guessing risks corrupting it"
        );
    }

    #[test]
    fn snapshot_omits_the_schema_version() {
        let (s, _) = with_docs(1);
        let json = serde_json::to_string(&SessionSnapshot::from(&s)).unwrap();
        assert!(!json.contains("schema_version"));
    }
}
