//! Region arrangement. See specs/001-app-shell/data-model.md.

use serde::{Deserialize, Serialize};

/// Below this an output console shows fewer than three lines. Regions are hidden rather
/// than shrunk past it. The tool window has its own minimum — see `domain::rail` — because
/// it is no longer one of these generic regions.
pub const MIN_REGION_EXTENT: u32 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionId {
    Output,
    DocumentArea,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionState {
    pub visible: bool,
    /// Pixels along the region's variable axis. Retained while hidden, so showing a region
    /// again restores its previous size rather than a default.
    pub extent: u32,
}

impl RegionState {
    pub fn new(visible: bool, extent: u32) -> Self {
        Self { visible, extent }
    }

    /// Repair on load: a stale file should not cost the user their session.
    /// Live commands reject instead — see `Layout::set_region`.
    pub fn clamped(self) -> Self {
        if self.visible && self.extent < MIN_REGION_EXTENT {
            Self {
                extent: MIN_REGION_EXTENT,
                ..self
            }
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub output: RegionState,
    pub document_area: RegionState,
    /// F000 modelled the left panel as a generic `navigation` region. F018 gave that
    /// position its real identity — the prototype's tool window — which owns its own width
    /// and collapsed state, so two descriptions of one surface existed.
    ///
    /// Read from older files so an existing session keeps its panel width, never written
    /// back, and folded into `tool_window` by `PersistedSession::repaired`. It disappears
    /// from the file on the next save.
    #[serde(default, rename = "navigation", skip_serializing)]
    pub legacy_navigation: Option<RegionState>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            output: RegionState::new(true, 200),
            document_area: RegionState::new(true, 0),
            legacy_navigation: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    ExtentBelowMinimum,
    DocumentAreaNotHideable,
}

impl Layout {
    pub fn get(&self, id: RegionId) -> RegionState {
        match id {
            RegionId::Output => self.output,
            RegionId::DocumentArea => self.document_area,
        }
    }

    /// Live mutation. Rejects rather than repairs: an out-of-range value here indicates an
    /// interface-layer defect, which should surface loudly.
    pub fn set_region(
        &mut self,
        id: RegionId,
        visible: bool,
        extent: u32,
    ) -> Result<(), LayoutError> {
        if id == RegionId::DocumentArea && !visible {
            return Err(LayoutError::DocumentAreaNotHideable);
        }
        if visible && extent < MIN_REGION_EXTENT {
            return Err(LayoutError::ExtentBelowMinimum);
        }
        let slot = match id {
            RegionId::Output => &mut self.output,
            RegionId::DocumentArea => &mut self.document_area,
        };
        *slot = RegionState::new(visible, extent);
        Ok(())
    }

    pub fn clamped(self) -> Self {
        Self {
            output: self.output.clamped(),
            // The primary area is not hideable; a persisted false is repaired, not honoured.
            document_area: RegionState {
                visible: true,
                ..self.document_area
            }
            .clamped(),
            // Deliberately carried through rather than cleared here: clamping is a repair
            // of region sizes, and consuming the legacy field is the session's job, which
            // is the only place that can see the tool window it folds into.
            legacy_navigation: self.legacy_navigation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_region_below_minimum_is_clamped_on_load() {
        let r = RegionState::new(true, 10).clamped();
        assert_eq!(r.extent, MIN_REGION_EXTENT);
    }

    #[test]
    fn hidden_region_retains_its_extent_through_clamping() {
        let r = RegionState::new(false, 10).clamped();
        assert_eq!(
            r.extent, 10,
            "hidden regions keep their size so showing restores it"
        );
    }

    #[test]
    fn live_command_rejects_extent_below_minimum_rather_than_clamping() {
        let mut l = Layout::default();
        assert_eq!(
            l.set_region(RegionId::Output, true, 10),
            Err(LayoutError::ExtentBelowMinimum)
        );
    }

    #[test]
    fn document_area_cannot_be_hidden() {
        let mut l = Layout::default();
        assert_eq!(
            l.set_region(RegionId::DocumentArea, false, 500),
            Err(LayoutError::DocumentAreaNotHideable)
        );
    }

    #[test]
    fn persisted_hidden_document_area_is_repaired_on_load() {
        let l = Layout {
            document_area: RegionState::new(false, 400),
            ..Layout::default()
        }
        .clamped();
        assert!(l.document_area.visible);
    }

    #[test]
    fn hiding_a_region_is_permitted_below_the_minimum() {
        let mut l = Layout::default();
        assert!(l.set_region(RegionId::Output, false, 0).is_ok());
    }
}
