//! Wire payload to use-case input.
//!
//! An inbound adapter and nothing more: it translates and carries no business rule. The one
//! thing it decides is that a path which does not parse is dropped rather than passed on --
//! which is not a rule so much as the boundary itself. `RelPath::parse` is the client's own
//! containment check, run independently of whatever the engine did (FR-014, Principle VI).

use apex_protocol::wire::{FileEvent as WireEvent, FileEventKind};

use crate::application::use_cases::apply_file_event::{Change, FileEvent};
use crate::domain::workspace::RelPath;

/// Translated events, and how many were refused at this boundary.
pub struct Translated {
    pub events: Vec<FileEvent>,
    pub refused: usize,
}

pub fn translate(wire: &[WireEvent]) -> Translated {
    let mut events = Vec::with_capacity(wire.len());
    let mut refused = 0;
    for raw in wire {
        let Ok(path) = RelPath::parse(&raw.relative_path) else {
            refused += 1;
            continue;
        };
        let is_directory = matches!(raw.kind, Some(apex_protocol::wire::EntryKind::Directory));
        let change = match raw.event {
            FileEventKind::Created => Change::Created {
                is_directory,
                size: raw.size.unwrap_or(0),
                modified: raw.modified.unwrap_or(0),
            },
            FileEventKind::Modified => Change::Modified {
                is_directory,
                size: raw.size.unwrap_or(0),
                modified: raw.modified.unwrap_or(0),
            },
            FileEventKind::Deleted => Change::Deleted,
            FileEventKind::Renamed => {
                // A rename carries a second path, and it is untrusted exactly as the first is.
                let Some(to) = raw.to_path.as_deref().and_then(|t| RelPath::parse(t).ok()) else {
                    refused += 1;
                    continue;
                };
                Change::Renamed { to }
            }
        };
        events.push(FileEvent { path, change });
    }
    Translated { events, refused }
}
