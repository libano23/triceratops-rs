//! Cancellation and progress for long kernel operations.
//!
//! Placeholder: no token is installed yet, so nothing is ever cancelled.

/// The running job was cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

/// Called every few thousand units of work: `Err` when the job was cancelled.
pub fn checkpoint(_count: u64) -> Result<(), Cancelled> {
    Ok(())
}
