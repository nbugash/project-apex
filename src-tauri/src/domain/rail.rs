//! Activity rail and tool window state. See specs/002-shell-chrome-fidelity/data-model.md.
//!
//! Type names shorten "activity rail" to "rail"; they refer to the surface the
//! specification calls the activity rail.

use serde::{Deserialize, Serialize};

/// Below this the prototype's tool window header truncates its own title, so the panel is
/// present but unreadable.
pub const MIN_TOOL_WINDOW_WIDTH: u32 = 180;

/// The prototype's tool window width, and the default before a user resizes.
const DEFAULT_TOOL_WINDOW_WIDTH: u32 = 276;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DestinationId(pub String);

impl DestinationId {
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    /// The owning feature is not built. The destination still renders in its position —
    /// omitting it would change the rail's proportions and therefore its fidelity.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailDestination {
    pub id: DestinationId,
    pub label: &'static str,
    pub icon: &'static str,
    pub availability: Availability,
    pub order: u32,
}

impl RailDestination {
    pub fn is_selectable(&self) -> bool {
        self.availability == Availability::Available
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailError {
    UnknownDestination,
    WidthBelowMinimum,
}

/// The static destination set, derived from the prototype.
///
/// Hand-authored rather than generated: a destination is behaviour — an identity, a label,
/// an availability state — not a measurement. What keeps the list honest is the fidelity
/// gate, since a missing or extra destination changes the rail's geometry.
pub struct RailCatalogue {
    destinations: Vec<RailDestination>,
}

impl Default for RailCatalogue {
    fn default() -> Self {
        use Availability::{Available, Unavailable};
        // Most destinations belong to features that do not exist yet. They are present and
        // unavailable so the rail matches the prototype's proportions from the outset.
        let destinations = vec![
            ("files", "Project", "ph-folder", Available),
            ("vcs", "Version control", "ph-git-branch", Unavailable),
            ("search", "Search", "ph-magnifying-glass", Unavailable),
            ("run", "Run", "ph-play", Unavailable),
            ("problems", "Problems", "ph-warning-circle", Unavailable),
            ("terminal", "Terminal", "ph-terminal-window", Unavailable),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (id, label, icon, availability))| RailDestination {
            id: DestinationId::new(id),
            label,
            icon,
            availability,
            order: i as u32,
        })
        .collect();
        Self { destinations }
    }
}

impl RailCatalogue {
    pub fn all(&self) -> &[RailDestination] {
        &self.destinations
    }

    pub fn find(&self, id: &DestinationId) -> Option<&RailDestination> {
        self.destinations.iter().find(|d| &d.id == id)
    }

    pub fn first_available(&self) -> Option<&RailDestination> {
        self.destinations.iter().find(|d| d.is_selectable())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolWindowState {
    pub active_destination_id: Option<DestinationId>,
    pub collapsed: bool,
    /// Retained while collapsed, so showing the panel again restores its previous size.
    pub width: u32,
}

impl Default for ToolWindowState {
    fn default() -> Self {
        Self {
            active_destination_id: None,
            collapsed: false,
            width: DEFAULT_TOOL_WINDOW_WIDTH,
        }
    }
}

impl ToolWindowState {
    /// Selecting the ACTIVE destination toggles collapse. Selecting an unavailable one
    /// changes nothing and is not an error: the destination is legitimately present, and
    /// the interface already shows it as unavailable.
    pub fn select(
        &mut self,
        id: &DestinationId,
        catalogue: &RailCatalogue,
    ) -> Result<(), RailError> {
        let destination = catalogue.find(id).ok_or(RailError::UnknownDestination)?;
        if !destination.is_selectable() {
            return Ok(());
        }
        if self.active_destination_id.as_ref() == Some(id) {
            self.collapsed = !self.collapsed;
        } else {
            self.active_destination_id = Some(id.clone());
            self.collapsed = false;
        }
        Ok(())
    }

    /// Live mutation: rejects rather than repairs. An out-of-range value here indicates an
    /// interface-layer defect, which should surface.
    pub fn resize(&mut self, width: u32) -> Result<(), RailError> {
        if width < MIN_TOOL_WINDOW_WIDTH {
            return Err(RailError::WidthBelowMinimum);
        }
        self.width = width;
        Ok(())
    }

    /// Load path only. A stale file should not cost the user their session, so a dangling
    /// destination falls back to the first available one and a sub-minimum width is clamped.
    pub fn repaired(mut self, catalogue: &RailCatalogue) -> Self {
        let dangling = self
            .active_destination_id
            .as_ref()
            .is_some_and(|id| catalogue.find(id).is_none());
        if dangling {
            self.active_destination_id = catalogue.first_available().map(|d| d.id.clone());
        }
        if self.width < MIN_TOOL_WINDOW_WIDTH {
            self.width = MIN_TOOL_WINDOW_WIDTH;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> RailCatalogue {
        RailCatalogue::default()
    }

    #[test]
    fn destination_order_is_contiguous_and_ids_unique() {
        let c = catalogue();
        let orders: Vec<u32> = c.all().iter().map(|d| d.order).collect();
        assert_eq!(orders, (0..c.all().len() as u32).collect::<Vec<_>>());

        let mut ids: Vec<&DestinationId> = c.all().iter().map(|d| &d.id).collect();
        let before = ids.len();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        ids.dedup();
        assert_eq!(ids.len(), before, "destination identifiers must be unique");
    }

    #[test]
    fn unavailable_destinations_are_present_not_omitted() {
        let c = catalogue();
        assert!(
            c.all().iter().any(|d| !d.is_selectable()),
            "the rail must include destinations whose features are unbuilt"
        );
    }

    #[test]
    fn selecting_an_unavailable_destination_changes_nothing() {
        let c = catalogue();
        let mut s = ToolWindowState::default();
        let unavailable = c.all().iter().find(|d| !d.is_selectable()).unwrap();
        assert_eq!(s.select(&unavailable.id, &c), Ok(()));
        assert!(
            s.active_destination_id.is_none(),
            "no change, and not an error"
        );
    }

    #[test]
    fn selecting_an_unknown_destination_is_rejected() {
        let c = catalogue();
        let mut s = ToolWindowState::default();
        assert_eq!(
            s.select(&DestinationId::new("ghost"), &c),
            Err(RailError::UnknownDestination)
        );
    }

    #[test]
    fn selecting_the_active_destination_toggles_collapse() {
        let c = catalogue();
        let mut s = ToolWindowState::default();
        let first = c.first_available().unwrap().id.clone();

        s.select(&first, &c).unwrap();
        assert!(!s.collapsed);
        s.select(&first, &c).unwrap();
        assert!(s.collapsed, "selecting the active destination collapses");
        s.select(&first, &c).unwrap();
        assert!(!s.collapsed, "and selecting it again restores");
    }

    #[test]
    fn width_survives_a_collapse_cycle() {
        let c = catalogue();
        let mut s = ToolWindowState::default();
        let first = c.first_available().unwrap().id.clone();
        s.resize(420).unwrap();
        s.select(&first, &c).unwrap();
        s.select(&first, &c).unwrap();
        assert!(s.collapsed);
        assert_eq!(
            s.width, 420,
            "collapsing must not discard the user's sizing"
        );
    }

    #[test]
    fn a_live_resize_below_the_minimum_is_rejected() {
        let mut s = ToolWindowState::default();
        assert_eq!(s.resize(10), Err(RailError::WidthBelowMinimum));
        assert_eq!(
            s.width, DEFAULT_TOOL_WINDOW_WIDTH,
            "state unchanged on rejection"
        );
    }

    #[test]
    fn a_stale_width_below_the_minimum_is_clamped_on_load() {
        let s = ToolWindowState {
            width: 10,
            ..ToolWindowState::default()
        }
        .repaired(&catalogue());
        assert_eq!(
            s.width, MIN_TOOL_WINDOW_WIDTH,
            "repair on load, reject on command"
        );
    }

    #[test]
    fn a_dangling_destination_falls_back_rather_than_discarding_the_session() {
        let c = catalogue();
        let s = ToolWindowState {
            active_destination_id: Some(DestinationId::new("removed-in-a-later-version")),
            ..ToolWindowState::default()
        }
        .repaired(&c);
        assert_eq!(
            s.active_destination_id,
            c.first_available().map(|d| d.id.clone())
        );
    }
}
