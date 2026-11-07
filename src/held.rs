//! The ordering rules of held inputs, as plain data: the `Retry` backlog and its settle rule.
//! The run (src/run.rs) attempts; this module decides what an attempt's result does to the
//! backlog. Unit tests below pin every rule.

use std::collections::VecDeque;

use crate::trace::Disposition;

/// What one step attempt gave, as far as the policy cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tried {
    /// Admitted and committed.
    Committed,
    /// Refused; `final_refusal` when a reason is final.
    Refused { final_refusal: bool },
}

/// A `Retry` backlog: at most `capacity` inputs, each with its attempts so far, oldest first.
#[derive(Debug)]
pub(crate) struct Backlog<T> {
    entries: VecDeque<(T, u32)>,
    capacity: usize,
}

impl<T> Default for Backlog<T> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<T> Backlog<T> {
    /// An empty backlog of `capacity` places, allocated once.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// A newly refused input (its first attempt, not final): to the back, or dropped when the
    /// backlog is full.
    pub(crate) fn offer(&mut self, input: T) -> Disposition {
        if self.entries.len() < self.capacity {
            self.entries.push_back((input, 1));
            Disposition::Backlogged {
                len: self.entries.len(),
            }
        } else {
            Disposition::BacklogFull {
                capacity: self.capacity,
            }
        }
    }

    /// The input to retry next and its attempts so far.
    pub(crate) fn front(&self) -> Option<(&T, u32)> {
        self.entries.front().map(|(t, n)| (t, *n))
    }

    /// Settles the attempt of the front entry: committed, final, or exhausted pops it and
    /// retrying goes on; refused again keeps it in place with one more attempt and retrying
    /// stops. Returns the disposition and whether to go on.
    pub(crate) fn settle(&mut self, tried: Tried, max_attempts: u32) -> (Disposition, bool) {
        let Some((_, attempts)) = self.entries.front_mut() else {
            return (Disposition::Consumed { final_refusal: false }, false);
        };
        let n = attempts.saturating_add(1);
        match tried {
            Tried::Committed => {
                self.entries.pop_front();
                (Disposition::Committed, true)
            }
            Tried::Refused {
                final_refusal: true,
            } => {
                self.entries.pop_front();
                (
                    Disposition::Consumed {
                        final_refusal: true,
                    },
                    true,
                )
            }
            Tried::Refused { .. } if n >= max_attempts => {
                self.entries.pop_front();
                (Disposition::Exhausted { attempts: n }, true)
            }
            Tried::Refused { .. } => {
                *attempts = n;
                (Disposition::Kept { attempts: n }, false)
            }
        }
    }

    /// Every held input, oldest first; the backlog is empty after.
    pub(crate) fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.entries.drain(..).map(|(t, _)| t)
    }
}

/// Settles the attempt of the input `Backpressure` holds (`attempts` so far): `None` when it is
/// no longer held.
pub(crate) const fn settle_pending(tried: Tried, attempts: u32) -> (Disposition, Option<u32>) {
    let n = attempts.saturating_add(1);
    match tried {
        Tried::Committed => (Disposition::Committed, None),
        Tried::Refused {
            final_refusal: true,
        } => (
            Disposition::Consumed {
                final_refusal: true,
            },
            None,
        ),
        Tried::Refused { .. } => (Disposition::Backpressured { attempts: n }, Some(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFUSED: Tried = Tried::Refused {
        final_refusal: false,
    };
    const FINAL: Tried = Tried::Refused {
        final_refusal: true,
    };

    /// One retry pass as the run does it: front, attempt, settle, stop when told.
    fn pass(b: &mut Backlog<u32>, max: u32, mut outcome: impl FnMut(u32) -> Tried) -> Vec<(u32, Disposition)> {
        let mut seen = Vec::new();
        while let Some((input, _)) = b.front() {
            let input = *input;
            let (d, go_on) = b.settle(outcome(input), max);
            seen.push((input, d));
            if !go_on {
                break;
            }
        }
        seen
    }

    #[test]
    fn offer_appends_until_full_then_reports_full_and_drops() {
        let mut b = Backlog::new(2);
        assert_eq!(b.offer(1), Disposition::Backlogged { len: 1 });
        assert_eq!(b.offer(2), Disposition::Backlogged { len: 2 });
        assert_eq!(b.offer(3), Disposition::BacklogFull { capacity: 2 });
        assert_eq!(b.drain().collect::<Vec<_>>(), [1, 2]);
        let mut none: Backlog<u32> = Backlog::new(0);
        assert_eq!(none.offer(9), Disposition::BacklogFull { capacity: 0 });
    }

    #[test]
    fn retry_goes_oldest_first_and_stops_at_the_first_refusal() {
        let mut b = Backlog::new(4);
        for i in [10, 11, 12, 13] {
            let _ = b.offer(i);
        }
        let seen = pass(&mut b, u32::MAX, |i| if i == 12 { REFUSED } else { Tried::Committed });
        assert_eq!(
            seen,
            [
                (10, Disposition::Committed),
                (11, Disposition::Committed),
                (12, Disposition::Kept { attempts: 2 }),
            ]
        );
        // 13 was not attempted: it is younger than the entry that stays, so order is kept.
        assert_eq!(b.len(), 2);
        assert_eq!(b.front(), Some((&12, 2)));
    }

    #[test]
    fn a_kept_entry_counts_attempts_and_is_exhausted_at_the_bound() {
        let mut b = Backlog::new(2);
        let _ = b.offer(1);
        let _ = b.offer(2);
        assert_eq!(pass(&mut b, 3, |_| REFUSED), [(1, Disposition::Kept { attempts: 2 })]);
        // Third attempt reaches max_attempts = 3: dropped, and retrying goes on with 2.
        assert_eq!(
            pass(&mut b, 3, |i| if i == 1 { REFUSED } else { Tried::Committed }),
            [(1, Disposition::Exhausted { attempts: 3 }), (2, Disposition::Committed)]
        );
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn a_final_refusal_consumes_and_retrying_goes_on() {
        let mut b = Backlog::new(3);
        for i in [1, 2, 3] {
            let _ = b.offer(i);
        }
        let seen = pass(&mut b, u32::MAX, |i| if i == 1 { FINAL } else { Tried::Committed });
        assert_eq!(
            seen,
            [
                (1, Disposition::Consumed { final_refusal: true }),
                (2, Disposition::Committed),
                (3, Disposition::Committed),
            ]
        );
    }

    #[test]
    fn an_empty_backlog_attempts_nothing() {
        let mut b: Backlog<u32> = Backlog::new(4);
        assert!(pass(&mut b, u32::MAX, |_| panic!("nothing to attempt")).is_empty());
    }

    #[test]
    fn a_pending_input_stays_until_it_commits_or_is_final() {
        assert_eq!(
            settle_pending(REFUSED, 1),
            (Disposition::Backpressured { attempts: 2 }, Some(2))
        );
        assert_eq!(settle_pending(Tried::Committed, 2), (Disposition::Committed, None));
        assert_eq!(
            settle_pending(FINAL, 5),
            (Disposition::Consumed { final_refusal: true }, None)
        );
        assert!(Disposition::Backpressured { attempts: 2 }.is_held());
        assert!(!Disposition::BacklogFull { capacity: 1 }.is_held());
    }
}
