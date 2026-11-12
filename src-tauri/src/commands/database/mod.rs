//! Database Manager command boundary.
//!
//! Commands receive typed metadata and capability documents. Engine-specific
//! SQL remains inside `crate::database::{mysql, postgres}` and is never sent
//! across the Tauri boundary.

pub mod actions;
pub mod capabilities;
pub mod discovery;
pub mod profiles;
pub mod tunnels;
