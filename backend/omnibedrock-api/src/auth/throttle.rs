//! Login throttling: exponential backoff per username+source after repeated
//! failures. In-memory by design — a restart clears it, so no lockout state
//! outlives the process to be weaponized against shared NATs.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Failures before the backoff kicks in.
const STRIKES_FREE: u32 = 5;
/// Backoff base and ceiling.
const BASE: Duration = Duration::from_secs(2);
const CAP: Duration = Duration::from_secs(15 * 60);
/// Forgotten window: counters expire after this much inactivity.
const PRUNE_AFTER: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub(super) struct LoginThrottle {
    attempts: Mutex<HashMap<String, Attempt>>,
}

#[derive(Clone, Copy)]
struct Attempt {
    strikes: u32,
    last: Instant,
}

impl LoginThrottle {
    /// Reject while the key sits inside its backoff window; `Ok(())` otherwise.
    pub(super) fn check(&self, key: &str) -> Result<(), Duration> {
        let mut attempts = self.lock();
        attempts.retain(|_, a| a.last.elapsed() < PRUNE_AFTER);
        let Some(a) = attempts.get(key).copied() else {
            return Ok(());
        };
        if a.strikes < STRIKES_FREE {
            return Ok(());
        }
        let shift = (a.strikes - STRIKES_FREE).min(9);
        let wait = BASE.saturating_mul(1 << shift).min(CAP);
        let elapsed = a.last.elapsed();
        if elapsed >= wait {
            Ok(())
        } else {
            Err(wait - elapsed)
        }
    }

    pub(super) fn record_failure(&self, key: &str) {
        let mut attempts = self.lock();
        let now = Instant::now();
        let entry = attempts.entry(key.to_owned()).or_insert(Attempt {
            strikes: 0,
            last: now,
        });
        entry.strikes = entry.strikes.saturating_add(1);
        entry.last = now;
    }

    pub(super) fn clear(&self, key: &str) {
        self.lock().remove(key);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Attempt>> {
        // A poisoned lock here must not take the whole login path down.
        self.attempts.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_starts_after_free_strikes_and_grows() {
        let t = LoginThrottle::default();
        let key = "u\0ip";
        // STRIKES_FREE attempts are free: after k failures (< STRIKES_FREE)
        // the next attempt goes through unchecked.
        for _ in 0..STRIKES_FREE - 1 {
            t.record_failure(key);
            assert!(
                t.check(key).is_ok(),
                "no delay before {STRIKES_FREE} strikes"
            );
        }
        // The failure that reaches the threshold arms the backoff.
        t.record_failure(key);
        assert!(
            matches!(t.check(key), Err(w) if w <= BASE && w > Duration::ZERO),
            "the attempt right after the threshold waits ~BASE"
        );
    }

    #[test]
    fn success_clears_and_keys_are_independent() {
        let t = LoginThrottle::default();
        let a = "a\0ip";
        let b = "b\0ip";
        for _ in 0..8 {
            t.record_failure(a);
        }
        assert!(t.check(a).is_err());
        assert!(t.check(b).is_ok(), "another key is unaffected");
        t.clear(a);
        assert!(t.check(a).is_ok());
    }
}
