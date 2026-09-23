//! Putting an engine on a remote host.
//!
//! Deployment runs beside the protocol channel, not through it: at deployment time there is no
//! engine to talk to, and §4.1 caps a frame at 1 MiB against an artifact measured in tens of
//! megabytes. It streams over the SSH connection F001 already holds.

pub mod embedded;
