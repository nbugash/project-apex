//! Window geometry and the attached-display rule. See specs/001-app-shell/data-model.md.

use serde::{Deserialize, Serialize};

pub const MIN_WINDOW_WIDTH: u32 = 800;
pub const MIN_WINDOW_HEIGHT: u32 = 600;

/// Height of the region that must remain reachable. A window whose title bar is off-screen
/// cannot be dragged back by the user, which is why that — not mere overlap — is the test.
const TITLE_BAR_HEIGHT: i32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            x: 100,
            y: 100,
            width: 1200,
            height: 800,
            maximized: false,
        }
    }
}

/// The working area of one attached display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DisplayBounds {
    fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x + self.width as i32
            && py >= self.y
            && py < self.y + self.height as i32
    }
}

impl WindowGeometry {
    fn with_minimums(self) -> Self {
        Self {
            width: self.width.max(MIN_WINDOW_WIDTH),
            height: self.height.max(MIN_WINDOW_HEIGHT),
            ..self
        }
    }

    /// True when any part of the title bar falls inside an attached display's working area.
    pub fn title_bar_reachable(&self, displays: &[DisplayBounds]) -> bool {
        displays.iter().any(|d| {
            let bar_left = self.x;
            let bar_right = self.x + self.width as i32 - 1;
            let bar_top = self.y;
            let bar_bottom = self.y + TITLE_BAR_HEIGHT - 1;
            let xs = [bar_left, bar_right];
            let ys = [bar_top, bar_bottom];
            xs.iter()
                .any(|&px| ys.iter().any(|&py| d.contains_point(px, py)))
        })
    }

    /// FR-009. Restored geometry is accepted only when its title bar is reachable; otherwise
    /// the default position on the first attached display is used.
    pub fn constrained_to(self, displays: &[DisplayBounds]) -> Self {
        let candidate = self.with_minimums();
        if candidate.title_bar_reachable(displays) {
            return candidate;
        }
        match displays.first() {
            Some(primary) => Self {
                x: primary.x + 100,
                y: primary.y + 100,
                ..WindowGeometry::default()
            },
            None => WindowGeometry::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primary() -> DisplayBounds {
        DisplayBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn geometry_on_an_attached_display_is_kept() {
        let g = WindowGeometry {
            x: 200,
            y: 150,
            width: 1000,
            height: 700,
            maximized: false,
        };
        assert_eq!(g.constrained_to(&[primary()]), g);
    }

    #[test]
    fn geometry_on_a_detached_display_falls_back_to_the_primary() {
        // Last docked to an external monitor to the right; it is gone now.
        let g = WindowGeometry {
            x: 3000,
            y: 200,
            width: 1000,
            height: 700,
            maximized: false,
        };
        let out = g.constrained_to(&[primary()]);
        assert!(
            out.title_bar_reachable(&[primary()]),
            "window must open somewhere reachable"
        );
        assert_ne!(out.x, 3000);
    }

    #[test]
    fn a_window_positioned_above_the_screen_is_recovered() {
        // Title bar off the top edge cannot be grabbed, even though the body overlaps.
        let g = WindowGeometry {
            x: 100,
            y: -400,
            width: 1000,
            height: 700,
            maximized: false,
        };
        let out = g.constrained_to(&[primary()]);
        assert!(out.y >= 0);
    }

    #[test]
    fn undersized_geometry_is_clamped_to_the_minimum() {
        let g = WindowGeometry {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            maximized: false,
        };
        let out = g.constrained_to(&[primary()]);
        assert_eq!(
            (out.width, out.height),
            (MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)
        );
    }

    #[test]
    fn with_no_attached_displays_the_default_is_used() {
        let g = WindowGeometry {
            x: 5000,
            y: 5000,
            width: 900,
            height: 700,
            maximized: false,
        };
        assert_eq!(g.constrained_to(&[]), WindowGeometry::default());
    }

    #[test]
    fn a_secondary_display_counts_as_attached() {
        let secondary = DisplayBounds {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let g = WindowGeometry {
            x: 2000,
            y: 100,
            width: 1000,
            height: 700,
            maximized: false,
        };
        assert_eq!(g.constrained_to(&[primary(), secondary]), g);
    }
}
