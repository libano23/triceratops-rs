//! Locust — a node-based algorithmic modeller (the counterpart of Grasshopper).
//!
//! Components are wired on a canvas; the graph is solved into data trees, the results are
//! previewed in the viewport, and Bake turns them into document objects. The core — data
//! trees, the graph, the engine and the component library — has no UI and is tested
//! headless; `ui` is the egui editor.

#[allow(dead_code)] // used by the MCP Locust tools (next commit)
pub mod authoring;
pub mod components;
pub mod data;
pub mod engine;
pub mod example;
pub mod graph;
pub mod ui;

#[cfg(test)]
mod tests;

