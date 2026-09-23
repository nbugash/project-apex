//! Window creation, display constraint, and the readiness gate.
//!
//! The window is created hidden and shown only on an explicit readiness signal. That is what
//! makes FR-020 ("no light or unstyled frame") a structural property rather than a race:
//! with the window hidden until styles have applied, SC-011 asserts a design guarantee
//! instead of a timing accident.

use crate::domain::geometry::{DisplayBounds, WindowGeometry};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{LogicalPosition, LogicalSize, Manager, WebviewWindow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLifecycle {
    Loading,
    Ready,
}

#[derive(Debug, thiserror::Error)]
pub enum WindowError {
    #[error("main window not found")]
    NoWindow,
    #[error("window operation failed: {0}")]
    Platform(String),
}

pub struct WindowController {
    window: WebviewWindow,
    ready: AtomicBool,
}

impl WindowController {
    pub fn from_app(app: &tauri::AppHandle) -> Result<Self, WindowError> {
        let window = app
            .get_webview_window("main")
            .ok_or(WindowError::NoWindow)?;
        Ok(Self {
            window,
            ready: AtomicBool::new(false),
        })
    }

    pub fn lifecycle(&self) -> SessionLifecycle {
        if self.ready.load(Ordering::SeqCst) {
            SessionLifecycle::Ready
        } else {
            SessionLifecycle::Loading
        }
    }

    /// The displays currently attached, as the domain sees them.
    pub fn attached_displays(&self) -> Vec<DisplayBounds> {
        self.window
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(|m| {
                let p = m.position();
                let s = m.size();
                DisplayBounds {
                    x: p.x,
                    y: p.y,
                    width: s.width,
                    height: s.height,
                }
            })
            .collect()
    }

    pub fn apply(&self, geometry: &WindowGeometry) -> Result<(), WindowError> {
        let fail = |e: tauri::Error| WindowError::Platform(e.to_string());
        self.window
            .set_size(LogicalSize::new(
                geometry.width as f64,
                geometry.height as f64,
            ))
            .map_err(fail)?;
        self.window
            .set_position(LogicalPosition::new(geometry.x as f64, geometry.y as f64))
            .map_err(fail)?;
        if geometry.maximized {
            self.window.maximize().map_err(fail)?;
        }
        Ok(())
    }

    pub fn current_geometry(&self) -> Option<WindowGeometry> {
        let pos = self.window.outer_position().ok()?;
        let size = self.window.inner_size().ok()?;
        Some(WindowGeometry {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized: self.window.is_maximized().unwrap_or(false),
        })
    }

    /// Idempotent: a second call is a no-op, not an error. Interface-layer reloads during
    /// development would otherwise fail spuriously.
    pub fn mark_ready(&self) -> Result<(), WindowError> {
        if self.ready.swap(true, Ordering::SeqCst) {
            crate::logging::info("readiness signal repeated; window already shown");
            return Ok(());
        }
        crate::logging::info("readiness signal received; showing window");
        self.window
            .show()
            .map_err(|e| WindowError::Platform(e.to_string()))?;
        self.window
            .set_focus()
            .map_err(|e| WindowError::Platform(e.to_string()))?;
        crate::logging::info("window shown");
        Ok(())
    }
}
