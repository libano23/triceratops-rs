//! Cancellation and progress for long kernel operations.
//!
//! A job installs a [`Token`] on the thread that does the work; deep kernel loops call
//! [`checkpoint`] every few thousand units of work, which records progress and returns
//! `Err(Cancelled)` once someone has cancelled the token. With no token installed nothing is
//! ever cancelled, so code outside a job behaves as before.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The running job was cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cancelled")
    }
}

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    count: AtomicU64,
    stage: Mutex<&'static str>,
    notes: Mutex<Vec<String>>,
}

/// How many distinct notes a job keeps.
const MAX_NOTES: usize = 8;

/// Shared between the thread doing the work and whoever watches or cancels it.
#[derive(Clone, Default)]
pub struct Token(Arc<Inner>);

impl Token {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Relaxed)
    }

    /// The notes the work left (why a step inside it was refused), each once, in order; taking
    /// them empties the list.
    pub fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.notes.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// What the work is doing now and how far it got (units depend on the stage).
    pub fn snapshot(&self) -> (&'static str, u64) {
        (*self.0.stage.lock().unwrap_or_else(|e| e.into_inner()), self.0.count.load(Ordering::Relaxed))
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Token>> = const { RefCell::new(None) };
}

/// Puts back the token that was installed before, when dropped.
pub struct Guard(Option<Token>);

impl Drop for Guard {
    fn drop(&mut self) {
        let prev = self.0.take();
        CURRENT.with(|c| *c.borrow_mut() = prev);
    }
}

/// Make `t` the current thread's token until the guard is dropped.
pub fn install(t: Token) -> Guard {
    Guard(CURRENT.with(|c| c.borrow_mut().replace(t)))
}

/// The token of the job running on this thread, if any.
pub fn current() -> Option<Token> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Leave a note for whoever owns the running job (shown in the log when it ends): a step the
/// work skipped, and why. No job, no note. The same text is kept once, and only a few are kept.
pub fn note(msg: impl Into<String>) {
    if let Some(t) = current() {
        let msg = msg.into();
        let mut notes = t.0.notes.lock().unwrap_or_else(|e| e.into_inner());
        if notes.len() < MAX_NOTES && !notes.contains(&msg) {
            notes.push(msg);
        }
    }
}

/// Name the stage the work is in (shown to the user) and restart its counter.
pub fn stage(name: &'static str) {
    if let Some(t) = current() {
        *t.0.stage.lock().unwrap_or_else(|e| e.into_inner()) = name;
        t.0.count.store(0, Ordering::Relaxed);
    }
}

/// Record progress; `Err` when the job was cancelled.
pub fn checkpoint(count: u64) -> Result<(), Cancelled> {
    match current() {
        Some(t) => {
            t.0.count.store(count, Ordering::Relaxed);
            if t.is_cancelled() {
                Err(Cancelled)
            } else {
                Ok(())
            }
        }
        None => Ok(()),
    }
}

/// `Err` when the current job was cancelled (no progress recorded).
pub fn check() -> Result<(), Cancelled> {
    if current().is_some_and(|t| t.is_cancelled()) {
        Err(Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_sees_cancel() {
        assert_eq!(checkpoint(1), Ok(()), "no token: never cancelled");
        let t = Token::new();
        let _g = install(t.clone());
        stage("testing");
        assert_eq!(checkpoint(42), Ok(()));
        assert_eq!(t.snapshot(), ("testing", 42));
        t.cancel();
        assert_eq!(checkpoint(43), Err(Cancelled));
        assert_eq!(check(), Err(Cancelled));
    }

    #[test]
    fn notes_reach_the_owner_of_the_token_once_each() {
        let t = Token::new();
        {
            let _g = install(t.clone());
            note("a Boolean was refused: x");
            note("a Boolean was refused: x");
            note("another");
        }
        assert_eq!(t.take_notes(), vec!["a Boolean was refused: x".to_string(), "another".to_string()]);
        assert!(t.take_notes().is_empty(), "taking them empties the list");
    }

    #[test]
    fn a_note_outside_a_job_is_dropped_quietly() {
        note("nobody is listening");
    }

    #[test]
    fn notes_are_capped() {
        let t = Token::new();
        let _g = install(t.clone());
        for i in 0..100 {
            note(format!("n{i}"));
        }
        assert_eq!(t.take_notes().len(), 8);
    }

    #[test]
    fn guard_restores_previous_token() {
        let a = Token::new();
        let _ga = install(a.clone());
        {
            let b = Token::new();
            let _gb = install(b.clone());
            b.cancel();
            assert_eq!(check(), Err(Cancelled));
        }
        assert_eq!(check(), Ok(()), "outer token is back");
    }

    #[test]
    fn token_crosses_big_stack() {
        let t = Token::new();
        let _g = install(t.clone());
        t.cancel();
        let r = crate::mesh_csg::with_big_stack(check).unwrap();
        assert_eq!(r, Err(Cancelled), "the Boolean thread sees the caller's token");
    }
}
