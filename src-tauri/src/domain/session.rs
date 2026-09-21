//! Session state. See specs/001-app-shell/data-model.md and
//! specs/001-app-shell/contracts/session-state.schema.json.

use super::geometry::{DisplayBounds, WindowGeometry};
use super::layout::Layout;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
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
    pub name: String,
    pub location_type: LocationType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenDocumentReference {
    pub id: DocumentId,
    pub display_name: String,
    pub order: u32,
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
}

impl From<&PersistedSession> for SessionSnapshot {
    fn from(p: &PersistedSession) -> Self {
        Self {
            workspace: p.workspace.clone(),
            window: p.window,
            layout: p.layout,
            documents: p.documents.clone(),
            focused_document_id: p.focused_document_id.clone(),
        }
    }
}

impl PersistedSession {
    /// Invariants that JSON Schema cannot express. A file failing any of these is discarded
    /// in full rather than partially recovered — a half-restored layout is harder to reason
    /// about than a default one.
    pub fn is_coherent(&self) -> bool {
        if self.schema_version != SCHEMA_VERSION {
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
        self
    }

    pub fn open_document(&mut self, display_name: &str) -> Result<DocumentId, SessionError> {
        let trimmed = display_name.trim();
        if trimmed.is_empty() || display_name.chars().count() > MAX_NAME {
            return Err(SessionError::InvalidDisplayName);
        }
        let id = DocumentId::new();
        self.documents.push(OpenDocumentReference {
            id: id.clone(),
            display_name: display_name.to_string(),
            order: self.documents.len() as u32,
        });
        self.focused_document_id = Some(id.clone());
        Ok(id)
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
            .map(|i| s.open_document(&format!("doc{i}")).unwrap())
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
            s.open_document("   "),
            Err(SessionError::InvalidDisplayName)
        );
        assert_eq!(
            s.open_document(&"x".repeat(256)),
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
    fn snapshot_omits_the_schema_version() {
        let (s, _) = with_docs(1);
        let json = serde_json::to_string(&SessionSnapshot::from(&s)).unwrap();
        assert!(!json.contains("schema_version"));
    }
}
