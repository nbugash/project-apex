//! What the projection holds, and what the interface is told about it.
//!
//! Entity definitions live in specs/005-workspace-cache/data-model.md.

use super::workspace::{FileId, RelPath, Sha256};

/// The application-layer view of one cached file. Not a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub file_id: FileId,
    pub path: RelPath,
    /// Of the decompressed content (§5.6).
    pub hash: Sha256,
    pub bytes: Vec<u8>,
    pub last_accessed_at: i64,
}

/// Whether cached content may be served.
///
/// FR-019 as a type: cached content is valid **exactly** when its hash matches the engine's, and
/// nothing else invalidates it. There is deliberately **one** constructor, and it takes two
/// hashes. In particular there is no path by which git status can reach this decision — §5.3 and
/// FR-020 make that a rule, and a type with no constructor accepting it is what enforces the rule
/// rather than a comment asking people to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validity {
    Valid,
    Stale,
    Absent,
}

impl Validity {
    /// The only way to construct one.
    pub fn compare(cached: Option<&Sha256>, engine: Option<&Sha256>) -> Self {
        match (cached, engine) {
            (Some(c), Some(e)) if c == e => Self::Valid,
            (Some(_), Some(_)) => Self::Stale,
            (None, _) => Self::Absent,
            // The engine could not say — a directory, or a stat that produced no digest. Not a
            // match, so not servable as current.
            (Some(_), None) => Self::Stale,
        }
    }
}

/// What the interface is told about content it is about to show.
///
/// Five of the six qualify bytes. `Gone` does not: it says the thing being projected does not
/// exist, so there is nothing to qualify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// Connected, confirmation outstanding (FR-021b).
    Verifying,
    /// Confirmed against the engine this open (FR-021a).
    Current,
    /// The confirmation exceeded its limit (FR-021c).
    Unverified,
    /// Served while disconnected (FR-032).
    PossiblyStale,
    /// Disconnected and not cached (FR-033).
    Unavailable,
    /// The workspace's root no longer exists on the engine (FR-038).
    Gone,
}

impl Presentation {
    /// Every variant, so a renderer can be checked exhaustively — the greyscale test (SC-016)
    /// needs the full set rather than whichever ones a fixture happened to produce.
    pub const ALL: [Self; 6] = [
        Self::Verifying,
        Self::Current,
        Self::Unverified,
        Self::PossiblyStale,
        Self::Unavailable,
        Self::Gone,
    ];
}

/// Startup maintenance, published at least once per second while it runs (FR-018a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintenancePhase {
    /// Before maintenance begins.
    Idle,
    /// Reading the schema version. Too brief to be worth rendering.
    Checking,
    /// A schema step is running (FR-018a).
    Migrating { from: u32, to: u32 },
    /// A migration failed; the projection is being discarded and recreated (FR-018b).
    Rebuilding,
    /// Retention is reclaiming content.
    Evicting,
    /// Complete. **The precondition for constructing any provider** (FR-018c).
    Ready,
}

impl MaintenancePhase {
    /// Whether the interface shows this phase. `Idle`, `Checking` and `Ready` are not rendered.
    pub fn is_rendered(&self) -> bool {
        matches!(
            self,
            Self::Migrating { .. } | Self::Rebuilding | Self::Evicting
        )
    }
}

/// How long content survives unopened (§5.5, FR-026).
///
/// A value rather than a constant so eviction is testable against a fake clock without waiting a
/// fortnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionWindow(pub i64);

impl RetentionWindow {
    pub const DAYS_14: Self = Self(14 * 24 * 60 * 60);

    /// The cutoff: content not accessed since this instant is evictable.
    pub fn cutoff(&self, now: i64) -> i64 {
        now - self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(b: &[u8]) -> Sha256 {
        Sha256::of(b)
    }

    #[test]
    fn validity_is_a_hash_comparison_and_nothing_else() {
        let a = h(b"same");
        let b = h(b"same");
        let c = h(b"different");
        assert_eq!(Validity::compare(Some(&a), Some(&b)), Validity::Valid);
        assert_eq!(Validity::compare(Some(&a), Some(&c)), Validity::Stale);
        assert_eq!(Validity::compare(None, Some(&c)), Validity::Absent);
        assert_eq!(
            Validity::compare(Some(&a), None),
            Validity::Stale,
            "an engine that gave no digest has not confirmed anything, so the cached copy is \
             not servable as current"
        );
    }

    #[test]
    fn exactly_three_maintenance_phases_are_rendered() {
        let all = [
            MaintenancePhase::Idle,
            MaintenancePhase::Checking,
            MaintenancePhase::Migrating { from: 0, to: 1 },
            MaintenancePhase::Rebuilding,
            MaintenancePhase::Evicting,
            MaintenancePhase::Ready,
        ];
        let rendered: Vec<_> = all.iter().filter(|p| p.is_rendered()).cloned().collect();
        assert_eq!(
            rendered,
            vec![
                MaintenancePhase::Migrating { from: 0, to: 1 },
                MaintenancePhase::Rebuilding,
                MaintenancePhase::Evicting
            ],
            "FR-018a needs migrating distinguishable from evicting; collapsing them would make \
             SC-013a unmeasurable"
        );
    }

    #[test]
    fn the_retention_window_is_fourteen_days() {
        let w = RetentionWindow::DAYS_14;
        assert_eq!(w.0, 1_209_600);
        assert_eq!(w.cutoff(1_209_600), 0);
    }

    #[test]
    fn every_presentation_variant_is_listed_in_all() {
        // The greyscale gate (SC-016) asserts over ALL. If a variant is added and forgotten
        // here, that gate would silently stop covering it.
        assert_eq!(Presentation::ALL.len(), 6);
        for v in Presentation::ALL {
            assert!(Presentation::ALL.contains(&v));
        }
    }
}
