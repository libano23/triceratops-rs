//! Requests and replies between `triceratops mcp` and the running window: one JSON
//! object per line. The format is private to the two halves of this executable; the
//! first exchange is always `hello`, carrying the version, so mismatched halves refuse
//! each other.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self { id, result: Some(result), error: None }
    }

    pub fn err(id: u64, error: impl Into<String>) -> Self {
        Self { id, result: None, error: Some(error.into()) }
    }

    pub fn into_result(self) -> Result<Value, String> {
        match (self.result, self.error) {
            (_, Some(e)) => Err(e),
            (Some(v), None) => Ok(v),
            (None, None) => Ok(Value::Null),
        }
    }
}
