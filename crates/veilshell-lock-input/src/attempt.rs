//! Credential-free retry and cancellation state for one lock lifetime.
//!
//! The caller owns the lock, editor, and trusted authentication worker. A
//! positive result is meaningful only after both PAM authentication and account
//! validation, and only while the compositor lock and editor remain eligible.

use std::time::{Duration, Instant};

/// Ticket passed privately to the authentication worker and returned with its result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

/// State for one lock lifetime. Do not reuse across locks or share workers
/// across separate instances.
#[derive(Default)]
pub struct Attempts {
    sequence: u64,
    pending: Option<(Ticket, bool)>,
    retry_after: Option<Instant>,
}

const RETRY_DELAY: Duration = Duration::from_secs(2);

impl Attempts {
    /// Reserve the single worker slot. The caller must report even a submission
    /// failure as a failed completion of this ticket.
    #[must_use]
    pub fn begin(&mut self, now: Instant) -> Option<Ticket> {
        if self.pending.is_some() || self.retry_after.is_some_and(|until| now < until) {
            return None;
        }
        self.sequence = self.sequence.checked_add(1)?;
        let ticket = Ticket(self.sequence);
        self.pending = Some((ticket, false));
        Some(ticket)
    }

    /// Invalidate the response without freeing the worker slot. Call on
    /// cancellation, loss of the input stream or lock surface, or reconfiguration.
    pub fn cancel(&mut self) {
        if let Some((_, cancelled)) = &mut self.pending {
            *cancelled = true;
        }
    }

    /// Returns true only for a current, uncancelled success from the trusted
    /// worker. The caller must still check compositor and UI eligibility.
    /// A stale result may neither grant success nor consume a newer attempt.
    #[must_use]
    pub fn complete(&mut self, ticket: Ticket, authenticated: bool, now: Instant) -> bool {
        let Some((expected, cancelled)) = self.pending else {
            return false;
        };
        if ticket != expected {
            return false;
        }
        self.pending = None;
        self.retry_after = Some(now + RETRY_DELAY);
        authenticated && !cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_success_is_accepted_once() {
        let now = Instant::now();
        let mut attempts = Attempts::default();
        let current = attempts.begin(now).unwrap();
        assert!(attempts.complete(current, true, now));
        assert!(!attempts.complete(current, true, now));
    }

    #[test]
    fn cancellation_invalidates_success_and_keeps_the_worker_serial() {
        let now = Instant::now();
        let mut attempts = Attempts::default();
        let cancelled = attempts.begin(now).unwrap();
        attempts.cancel();
        assert!(attempts.begin(now + RETRY_DELAY).is_none());
        assert!(!attempts.complete(cancelled, true, now + RETRY_DELAY));
        assert!(attempts.begin(now + RETRY_DELAY).is_none());
        let current = attempts.begin(now + 2 * RETRY_DELAY).unwrap();
        assert!(!attempts.complete(cancelled, true, now + 2 * RETRY_DELAY));
        assert!(attempts.complete(current, true, now + 2 * RETRY_DELAY));
    }

    #[test]
    fn denial_waits_from_completion_and_cannot_be_replayed() {
        let now = Instant::now();
        let mut attempts = Attempts::default();
        let first = attempts.begin(now).unwrap();
        let completed = now + Duration::from_secs(30);
        assert!(!attempts.complete(first, false, completed));
        let before_deadline = (completed + RETRY_DELAY)
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        assert!(attempts.begin(before_deadline).is_none());
        let second = attempts.begin(completed + RETRY_DELAY).unwrap();
        assert!(!attempts.complete(first, true, completed + RETRY_DELAY));
        assert!(attempts.complete(second, true, completed + RETRY_DELAY));
    }
}
