//! The logic behind `cargo remedy` and the `remedy-mcp` server: the developer tools, the app
//! templates and the MCP server, each a module behind a Cargo feature of the same name.

#[cfg(feature = "devkit")]
pub mod devkit;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "templates")]
pub mod templates;
