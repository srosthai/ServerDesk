//! Database Manager command boundary.
//!
//! Commands receive typed metadata and capability documents. Engine-specific
//! SQL remains inside `crate::database::{mysql, postgres}` and is never sent
//! across the Tauri boundary.

pub mod actions;
pub mod capabilities;
pub mod csv;
pub mod discovery;
pub mod dump;
pub mod health;
pub mod history;
pub mod objects;
pub mod profiles;
pub mod queries;
pub mod roles;
pub mod row_writes;
pub mod rows;
pub mod sessions;
pub mod structure;
pub mod tunnels;
