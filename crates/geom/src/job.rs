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
}

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
