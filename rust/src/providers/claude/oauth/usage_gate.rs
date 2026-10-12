//! Gate in front of the Claude OAuth usage endpoint (nesszer/Win-CodexBar#775).
//!
//! * After a 429 no request goes out until
//!   `max(Retry-After.min(60 min), (5 min * 2^(n-1)).min(30 min))` has passed.
//!   `n` counts consecutive 429s and resets only on a success from the sign-in
//!   that was blocked, never because the block expired or because another
//!   account succeeded. The block itself ignores the token, so a token refresh
//!   does not lift it. Concurrent 429s while a block is active do not escalate.
//! * The state lives in one small JSON file under `%LOCALAPPDATA%\CodexBar\`,
//!   so every CodexBar process of the user (tray app and one-off
//!   `codexbar usage` runs) shares it. An in-memory copy keeps the gate working
//!   when the file cannot be read or written.
//! * The last successful response body is kept in the same file and served again
//!   for 3 minutes to the same sign-in. The response holds only utilization
//!   percentages, reset timestamps and credit amounts (see `OAuthUsageResponse`
//!   in `oauth/mod.rs`), no token or account identity. Precedent: the CLI probe
//!   screen is persisted across processes the same way. The sign-in is keyed by
//!   the first 16 hex characters of the SHA-256 of the access token; the token
//!   itself is never written.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

const BASE_BACKOFF_MS: i64 = 5 * 60 * 1000;
/// Cap of the exponential ramp.
const MAX_BACKOFF_MS: i64 = 30 * 60 * 1000;
/// Cap of an honored `Retry-After`; also the longest wait ever reported.
const MAX_RETRY_AFTER_MS: i64 = 60 * 60 * 1000;
const MIN_POLL_INTERVAL_MS: i64 = 3 * 60 * 1000;
const FILE_NAME: &str = "claude-oauth-usage-gate.json";

/// What the caller should do before hitting the endpoint.
#[derive(Debug, PartialEq)]
pub(super) enum Decision {
    Blocked(Duration),
    /// Raw body of a recent successful response, to be parsed by the caller.
    Cached(String),
    Proceed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct CachedBody {
    at_ms: i64,
    fp: String,
    body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Persisted {
    blocked_until_ms: i64,
    consecutive: u32,
    /// Fingerprint of the sign-in whose 429 started the current streak.
    #[serde(default)]
    blocked_fp: String,
    #[serde(default)]
    last_success: Option<CachedBody>,
}

pub(super) struct UsageGate {
    path: Option<PathBuf>,
    state: Persisted,
}

/// First 16 hex characters of the token's SHA-256. Never the token.
pub(super) fn token_fingerprint(token: &str) -> String {
    let mut hex = crate::core::sha256_hex(token.as_bytes());
    hex.truncate(16);
    hex
}

pub(super) fn default_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("CodexBar").join(FILE_NAME))
}

pub(super) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Anthropic often returns `Retry-After: 0` or `1` on the usage endpoint.
/// Honoring that literally re-hits 429 on the next poll and, after a
/// last-good miss, the tray maps the generic OAuth error to sign-in; the
/// exponential ramp is therefore the floor.
fn backoff_ms(retry_after: Duration, consecutive: u32) -> i64 {
    let exponential = BASE_BACKOFF_MS.saturating_mul(1i64 << consecutive.saturating_sub(1).min(10));
    let retry_after = i64::try_from(retry_after.as_millis()).unwrap_or(i64::MAX);
    retry_after
        .min(MAX_RETRY_AFTER_MS)
        .max(exponential.min(MAX_BACKOFF_MS))
}

impl UsageGate {
    pub(super) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            state: Persisted::default(),
        }
    }

    /// Replace memory with the shared file, which any process may have updated.
    fn refresh(&mut self) {
        let Some(path) = &self.path else { return };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        if let Ok(disk) = serde_json::from_str::<Persisted>(&text) {
            self.state = disk;
        }
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let Ok(json) = serde_json::to_vec(&self.state) else {
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            tracing::debug!("Claude OAuth usage gate directory not created: {err}");
            return;
        }
        if let Err(err) = crate::atomic_file::write_atomic(path, &json) {
            tracing::debug!("Claude OAuth usage gate not persisted: {err}");
        }
    }

    /// Time left on the active block, clamped so a skewed or hand-edited
    /// timestamp cannot lock requests out for longer than any real backoff.
    fn remaining(&self, now_ms: i64) -> Option<Duration> {
        let left = self.state.blocked_until_ms.saturating_sub(now_ms);
        (left > 0).then(|| Duration::from_millis(left.min(MAX_RETRY_AFTER_MS) as u64))
    }

    pub(super) fn check(&mut self, now_ms: i64, fingerprint: &str) -> Decision {
        self.refresh();
        if let Some(remaining) = self.remaining(now_ms) {
            return Decision::Blocked(remaining);
        }
        if let Some(last) = &self.state.last_success
            && last.fp == fingerprint
            && now_ms >= last.at_ms
            && now_ms - last.at_ms < MIN_POLL_INTERVAL_MS
        {
            return Decision::Cached(last.body.clone());
        }
        Decision::Proceed
    }

    /// Record a 429 and return how long requests are blocked.
    pub(super) fn record_rate_limit(
        &mut self,
        now_ms: i64,
        fingerprint: &str,
        retry_after: Duration,
    ) -> Duration {
        self.refresh();
        if let Some(remaining) = self.remaining(now_ms) {
            // Another request already escalated; do not count this 429 again.
            return remaining;
        }
        self.state.consecutive = self.state.consecutive.saturating_add(1);
        self.state.blocked_fp = fingerprint.to_string();
        let backoff = backoff_ms(retry_after, self.state.consecutive);
        self.state.blocked_until_ms = now_ms.saturating_add(backoff);
        self.persist();
        Duration::from_millis(backoff as u64)
    }

    /// Remember a successful response body. The 429 streak resets only when
    /// the sign-in that was blocked succeeds. The refresh is needed so this
    /// write does not overwrite a block another process just recorded.
    pub(super) fn record_success(&mut self, now_ms: i64, fingerprint: &str, body: String) {
        self.refresh();
        if self.state.blocked_fp == fingerprint {
            self.state.consecutive = 0;
            self.state.blocked_until_ms = 0;
            self.state.blocked_fp.clear();
        }
        self.state.last_success = Some(CachedBody {
            at_ms: now_ms,
            fp: fingerprint.to_string(),
            body,
        });
        self.persist();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60 * 1000;
    const NOW: i64 = 1_000_000_000_000;

    fn gate(dir: &tempfile::TempDir) -> UsageGate {
        UsageGate::new(Some(dir.path().join(FILE_NAME)))
    }

    fn blocked_mins(d: Decision) -> u64 {
        match d {
            Decision::Blocked(d) => d.as_secs().div_ceil(60),
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn consecutive_429s_escalate_even_after_the_block_expired() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = gate(&dir);
        let mut now = NOW;
        for expected in [5, 10, 20, 30, 30] {
            let wait = g.record_rate_limit(now, "fp", Duration::from_secs(1));
            assert_eq!(wait.as_secs(), expected * 60);
            now += i64::try_from(expected).unwrap() * MIN + 1;
            assert_eq!(g.check(now, "fp"), Decision::Proceed);
        }
    }

    #[test]
    fn retry_after_wins_when_longer_and_is_capped_at_an_hour() {
        let wait = |secs| {
            gate(&tempfile::tempdir().unwrap())
                .record_rate_limit(NOW, "fp", Duration::from_secs(secs))
                .as_secs()
        };
        assert_eq!(wait(12 * 60), 720);
        assert_eq!(wait(45 * 60), 45 * 60);
        assert_eq!(wait(3 * 3600), 3600);
    }

    #[test]
    fn concurrent_429_during_an_active_block_does_not_escalate() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = gate(&dir);
        g.record_rate_limit(NOW, "fp", Duration::ZERO);
        let again = g.record_rate_limit(NOW + MIN, "fp", Duration::ZERO);
        assert_eq!(again.as_secs(), 4 * 60);
        // Block expires; the next 429 is the second of the streak.
        assert_eq!(
            g.record_rate_limit(NOW + 6 * MIN, "fp", Duration::ZERO)
                .as_secs(),
            600
        );
    }

    #[test]
    fn success_of_the_blocked_sign_in_resets_the_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = gate(&dir);
        g.record_rate_limit(NOW, "fp", Duration::ZERO);
        g.record_rate_limit(NOW + 6 * MIN, "fp", Duration::ZERO);
        g.record_success(NOW + 20 * MIN, "fp", "ok".into());
        assert_eq!(
            g.record_rate_limit(NOW + 30 * MIN, "fp", Duration::ZERO)
                .as_secs(),
            300
        );
    }

    #[test]
    fn success_of_another_sign_in_keeps_the_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = gate(&dir);
        g.record_rate_limit(NOW, "fp-a", Duration::ZERO);
        g.record_success(NOW + 6 * MIN, "fp-b", "ok".into());
        assert_eq!(
            g.record_rate_limit(NOW + 7 * MIN, "fp-a", Duration::ZERO)
                .as_secs(),
            600
        );
    }

    #[test]
    fn block_is_shared_through_the_file_and_ignores_the_token() {
        let dir = tempfile::tempdir().unwrap();
        gate(&dir).record_rate_limit(NOW, "fp", Duration::ZERO);
        let mut other = gate(&dir);
        assert_eq!(blocked_mins(other.check(NOW + MIN, "another-token")), 4);
        assert_eq!(
            other.check(NOW + 5 * MIN + 1, "another-token"),
            Decision::Proceed
        );
        // The count survives in the file, so the next 429 doubles.
        assert_eq!(
            other
                .record_rate_limit(NOW + 6 * MIN, "fp", Duration::ZERO)
                .as_secs(),
            600
        );
    }

    #[test]
    fn skewed_block_timestamp_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let state = Persisted {
            blocked_until_ms: NOW + 1000 * 60 * MIN,
            consecutive: 1,
            ..Persisted::default()
        };
        std::fs::write(
            dir.path().join(FILE_NAME),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert_eq!(blocked_mins(gate(&dir).check(NOW, "fp")), 60);
    }

    #[test]
    fn memory_copy_works_without_a_usable_file() {
        let dir = tempfile::tempdir().unwrap();
        // The path's parent is a regular file, so reads and writes fail.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let mut g = UsageGate::new(Some(blocker.join(FILE_NAME)));
        g.record_rate_limit(NOW, "fp", Duration::ZERO);
        assert_eq!(blocked_mins(g.check(NOW + MIN, "fp")), 4);
        let mut none = UsageGate::new(None);
        none.record_rate_limit(NOW, "fp", Duration::ZERO);
        assert_eq!(blocked_mins(none.check(NOW, "fp")), 5);
    }

    #[test]
    fn missing_directory_is_created_and_state_is_shared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CodexBar").join("nested").join(FILE_NAME);
        UsageGate::new(Some(path.clone())).record_rate_limit(NOW, "fp", Duration::ZERO);
        assert!(path.is_file());
        let mut other = UsageGate::new(Some(path));
        assert_eq!(blocked_mins(other.check(NOW + MIN, "fp")), 4);
    }

    #[test]
    fn corrupt_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), b"{not json").unwrap();
        let mut g = gate(&dir);
        assert_eq!(g.check(NOW, "fp"), Decision::Proceed);
        g.record_rate_limit(NOW, "fp", Duration::ZERO);
        assert_eq!(blocked_mins(g.check(NOW, "fp")), 5);
    }

    #[test]
    fn successful_polls_are_throttled_per_sign_in_across_processes() {
        let dir = tempfile::tempdir().unwrap();
        gate(&dir).record_success(NOW, "fp-a", "usage-a".into());
        let mut other = gate(&dir);
        assert_eq!(
            other.check(NOW + 2 * MIN, "fp-a"),
            Decision::Cached("usage-a".into())
        );
        assert_eq!(other.check(NOW + 2 * MIN, "fp-b"), Decision::Proceed);
        assert_eq!(other.check(NOW + 3 * MIN, "fp-a"), Decision::Proceed);
    }

    #[test]
    fn file_holds_no_token_and_only_a_truncated_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = gate(&dir);
        let full = crate::core::sha256_hex(b"secret-token");
        let fp = token_fingerprint("secret-token");
        assert_eq!(fp.len(), 16);
        assert!(full.starts_with(&fp));
        g.record_rate_limit(NOW, &fp, Duration::ZERO);
        g.record_success(NOW, &fp, r#"{"fiveHour":{"utilization":12.0}}"#.into());
        let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(!text.contains("secret-token") && !text.contains(&full));
        assert!(text.contains(&fp));
    }
}
