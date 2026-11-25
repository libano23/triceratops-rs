//! Driving Triceratops from AI assistants through the Model Context Protocol.
//!
//! `triceratops mcp` ([`bridge`]) is an MCP server on stdin/stdout. It forwards each tool
//! call over a local socket ([`socket`]) to the running window, where [`handler`] executes
//! it against the `App` between frames. [`protocol`] is the private JSON format between
//! the two halves.

pub mod handler;
pub mod protocol;
pub mod socket;
