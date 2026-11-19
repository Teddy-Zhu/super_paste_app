//! Domain and persistence contracts, independent of the desktop and async runtime.
pub mod config;
pub mod content;
pub mod history;
pub mod statistics;
pub mod ai;
pub mod sync;
pub mod mcp;
pub mod access;
pub use config::*;
pub use content::*;
pub use history::*;
pub use statistics::*;
pub use ai::*;
pub use sync::*;
pub use mcp::*;
pub use access::*;

// Persisted identity is independent of the product name; changing it would
// select another data directory and credential namespace.
pub const APPLICATION_ID: &str = "com.v2dawn.superpaste.rs";
pub const APPLICATION_NAME: &str = "Super Paste";
pub const DATABASE_FILE: &str = "super-paste-rs.sqlite3";
pub const SQLITE_APPLICATION_ID: i32 = 0x53505253;
pub const SCHEMA_VERSION: i32 = 2;
pub const INITIAL_SCHEMA: &str = include_str!("../schema.sql");

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub String);
