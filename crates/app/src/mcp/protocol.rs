//! Requests and replies between `triceratops mcp` and the running window: one JSON
//! object per line. The format is private to the two halves of this executable; the
//! first exchange is always `hello`, carrying the version, so mismatched halves refuse
//! each other.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Sent by the window's socket threads to wake the GUI thread while they wait for a long
/// request, so it polls the job even when no frame is drawn (minimized window). Not answered.
pub const TICK: &str = "_tick";

/// The version both halves must agree on.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
    /// Set on an intermediate frame: the request is still running (a job), here is how far
    /// it got. The final reply follows with the same id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub progress: Option<Value>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self { id, result: Some(result), error: None, progress: None }
    }

    pub fn err(id: u64, error: impl Into<String>) -> Self {
        Self { id, result: None, error: Some(error.into()), progress: None }
    }

    /// An intermediate frame for a request that is still running.
    pub fn progress(id: u64, progress: Value) -> Self {
        Self { id, result: None, error: None, progress: Some(progress) }
    }

    pub fn is_progress(&self) -> bool {
        self.progress.is_some()
    }

    pub fn into_result(self) -> Result<Value, String> {
        match (self.result, self.error) {
            (_, Some(e)) => Err(e),
            (Some(v), None) => Ok(v),
            (None, None) => Ok(Value::Null),
        }
    }
}
