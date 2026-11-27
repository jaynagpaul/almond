use fsrs::{FSRS, ItemState, NextStates};

pub use fsrs::MemoryState;

use crate::card::{Rating, Review};
use crate::time::{days_between, plus_days};
use jiff::Timestamp;

/// What the scheduler knows about a card after its latest review.
///
/// This is a cache of [`Scheduler::fold`] over the card's reviews. Storing it
/// keeps the hot path to one FSRS step; the log can always rebuild it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardState {
    pub memory: MemoryState,
    pub last_review: Timestamp,
    pub due: Timestamp,
}

/// Days until due for each rating, shown on the answer buttons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preview {
    pub again: f32,
    pub hard: f32,
    pub good: f32,
    pub easy: f32,
}

#[derive(Debug)]
pub enum ScheduleError {
    Retention(f32),
    Parameters(fsrs::FSRSError),
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::Retention(r) => write!(f, "desired retention {r} is outside 0.7..=0.99"),
            ScheduleError::Parameters(e) => write!(f, "invalid FSRS parameters: {e}"),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// FSRS with the user's parameters and target retention. Pure: the same
/// history always gives the same schedule.
#[derive(Debug, Clone)]
pub struct Scheduler {
    fsrs: FSRS,
    retention: f32,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            fsrs: FSRS::default(),
            retention: 0.9,
        }
    }
}

impl Scheduler {
    /// Empty `parameters` selects the FSRS-7 defaults.
    ///
    /// # Errors
    /// If `desired_retention` is outside `0.7..=0.99` or FSRS rejects the parameters.
    pub fn new(parameters: &[f32], desired_retention: f32) -> Result<Self, ScheduleError> {
        if !(0.7..=0.99).contains(&desired_retention) {
            return Err(ScheduleError::Retention(desired_retention));
        }
        let fsrs = FSRS::new(parameters).map_err(ScheduleError::Parameters)?;
        Ok(Self {
            fsrs,
            retention: desired_retention,
        })
    }

    /// The card's state after `rating` at `at`.
    pub fn advance(&self, prev: Option<&CardState>, rating: Rating, at: Timestamp) -> CardState {
        let next = self.next_states(prev, at);
        let item = match rating {
            Rating::Again => next.again,
            Rating::Hard => next.hard,
            Rating::Good => next.good,
            Rating::Easy => next.easy,
        };
        CardState {
            memory: item.memory,
            last_review: at,
            due: plus_days(at, item.interval),
        }
    }

    pub fn preview(&self, prev: Option<&CardState>, at: Timestamp) -> Preview {
        let NextStates {
            again,
            hard,
            good,
            easy,
        } = self.next_states(prev, at);
        let days = |s: ItemState| s.interval;
        Preview {
            again: days(again),
            hard: days(hard),
            good: days(good),
            easy: days(easy),
        }
    }

    /// Rebuilds a card's state from its reviews, oldest first.
    pub fn fold<'a>(&self, reviews: impl IntoIterator<Item = &'a Review>) -> Option<CardState> {
        reviews.into_iter().fold(None, |state, r| {
            Some(self.advance(state.as_ref(), r.rating, r.at))
        })
    }

    fn next_states(&self, prev: Option<&CardState>, at: Timestamp) -> NextStates {
        let elapsed = prev.map_or(0.0, |p| days_between(at, p.last_review));
        self.fsrs
            .next_states_with_elapsed_days(prev.map(|p| p.memory), self.retention, elapsed)
            .expect("FSRS steps are finite for parameters accepted by Scheduler::new")
    }
}
