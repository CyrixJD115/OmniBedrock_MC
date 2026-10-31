//! Login throttling: exponential backoff per username+source after repeated
//! failures, plus an account-wide lockout once failures pile up from ANY
//! source — rotating IPs must not buy unlimited fresh guesses at one
//! account. In-memory by design — a restart clears it, so no lockout state
//! outlives the process to be weaponized against shared NATs (the price is
//! that a brute force attempt can deny the account for one backoff window;
//! the panel restart also clears that).

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
    /// username\0ip keyed backoff.
    attempts: Mutex<HashMap<String, Attempt>>,
    /// username keyed totals across every source.
    accounts: Mutex<HashMap<String, Attempt>>,
}

#[derive(Clone, Copy)]
struct Attempt {
    strikes: u32,
    last: Instant,
}

impl LoginThrottle {
    /// Reject while any key sits inside its backoff window; `Ok(())`
    /// otherwise. The returned wait covers the worst of the provided keys.
    pub(super) fn check(&self, keys: &[&str]) -> Result<(), Duration> {
        let mut attempts = self.lock(&self.attempts);
        let mut accounts = self.lock(&self.accounts);
        attempts.retain(|_, a| a.last.elapsed() < PRUNE_AFTER);
        accounts.retain(|_, a| a.last.elapsed() < PRUNE_AFTER);
        keys.iter()
            .filter_map(|k| waiting(&attempts, k).or_else(|| waiting(&accounts, k)))
            .max()
            .map_or(Ok(()), Err)
    }

    pub(super) fn record_failure(&self, keys: &[&str]) {
        let now = Instant::now();
        // The per-source key is keys[0]; the last element is the account
        // key, so both counters advance on every failure.
        let (Some(source), Some(account)) = (keys.first(), keys.last()) else {
            return;
        };
        let mut attempts = self.lock(&self.attempts);
        bump(&mut attempts, source, now);
        let mut accounts = self.lock(&self.accounts);
        bump(&mut accounts, account, now);
    }

    pub(super) fn clear(&self, keys: &[&str]) {
        let (Some(source), Some(account)) = (keys.first(), keys.last()) else {
            return;
        };
        self.lock(&self.attempts).remove(*source);
        self.lock(&self.accounts).remove(*account);
    }

    fn lock<'m>(
        &self,
        map: &'m Mutex<HashMap<String, Attempt>>,
    ) -> std::sync::MutexGuard<'m, HashMap<String, Attempt>> {
        // A poisoned lock here must not take the whole login path down.
        map.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The remaining backoff for one key, or `None` while it may proceed.
fn waiting(attempts: &HashMap<String, Attempt>, key: &str) -> Option<Duration> {
    let a = attempts.get(key).copied()?;
    if a.strikes < STRIKES_FREE {
        return None;
    }
    let shift = (a.strikes - STRIKES_FREE).min(9);
    let wait = BASE.saturating_mul(1 << shift).min(CAP);
    let elapsed = a.last.elapsed();
    (elapsed < wait).then_some(wait - elapsed)
}

fn bump(attempts: &mut HashMap<String, Attempt>, key: &str, now: Instant) {
    let entry = attempts.entry(key.to_owned()).or_insert(Attempt {
        strikes: 0,
        last: now,
    });
    entry.strikes = entry.strikes.saturating_add(1);
    entry.last = now;
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
            t.record_failure(&[key]);
            assert!(
                t.check(&[key]).is_ok(),
                "no delay before {STRIKES_FREE} strikes"
            );
        }
        // The failure that reaches the threshold arms the backoff.
        t.record_failure(&[key]);
        assert!(
            matches!(t.check(&[key]), Err(w) if w <= BASE && w > Duration::ZERO),
            "the attempt right after the threshold waits ~BASE"
        );
    }

    #[test]
    fn success_clears_and_keys_are_independent() {
        let t = LoginThrottle::default();
        let a = "a\0ip";
        let b = "b\0ip";
        for _ in 0..8 {
            t.record_failure(&[a]);
        }
        assert!(t.check(&[a]).is_err());
        assert!(t.check(&[b]).is_ok(), "another key is unaffected");
        t.clear(&[a]);
        assert!(t.check(&[a]).is_ok());
    }

    #[test]
    fn rotating_sources_cannot_outguess_one_account() {
        let t = LoginThrottle::default();
        // Keys mirror authenticate(): "{username}\0{ip}" per source and
        // "{username}\0account" for the account-wide counter.
        // Failures spread across distinct source IPs arm the ACCOUNT counter
        // even though every per-source counter stays strike-free: the fifth
        // account-wide failure starts the same exponential curve.
        for i in 0..4 {
            let source = format!("user\x0010.0.0.{i}");
            t.record_failure(&[source.as_str(), "user\0account"]);
            assert!(
                t.check(&[source.as_str(), "user\0account"]).is_ok(),
                "4 account-wide failures must stay under the curve"
            );
        }
        let fifth = ["user\x0010.0.0.99", "user\0account"];
        t.record_failure(&fifth);
        assert!(
            matches!(t.check(&fifth), Err(w) if w <= BASE && w > Duration::ZERO),
            "the 5th account-wide failure arms the backoff despite the fresh IP"
        );
        // A different account is untouched by the lockout...
        let other = ["other\x0010.255.255.255", "other\0account"];
        assert!(t.check(&other).is_ok(), "another account is unaffected");
        // ...and a successful login clears both counters.
        t.clear(&fifth);
        assert!(t.check(&fifth).is_ok());
    }
}
