use std::collections::VecDeque;

use crate::card::{Card, ReviewId};
use crate::schedule::{CardState, Preview};
use crate::time::Timestamp;

/// A card waiting to be reviewed, with its cached schedule (`None` if new).
#[derive(Debug, Clone, PartialEq)]
pub struct Due {
    pub card: Card,
    pub state: Option<CardState>,
}

/// One review session. The phase makes the flow explicit: a card can only be
/// rated from `Back`, and `Done` has no current card.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Session {
    pub queue: VecDeque<Due>,
    pub reviewed: u32,
    pub phase: Phase,
    pub undo: Option<Undo>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Phase {
    Front {
        current: Due,
        shown_at: Timestamp,
    },
    Back {
        current: Due,
        shown_at: Timestamp,
        preview: Preview,
    },
    Done,
}

/// Enough to put the last rated card back on screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Undo {
    pub review: ReviewId,
    pub current: Due,
    pub preview: Preview,
}

impl Session {
    pub fn start(due: Vec<Due>, now: Timestamp) -> Self {
        let mut session = Self {
            queue: due.into(),
            reviewed: 0,
            phase: Phase::Done,
            undo: None,
        };
        session.show_next(now);
        session
    }

    pub fn show_next(&mut self, now: Timestamp) {
        self.phase = match self.queue.pop_front() {
            Some(current) => Phase::Front {
                current,
                shown_at: now,
            },
            None => Phase::Done,
        };
    }

    pub fn remaining(&self) -> u32 {
        let current = u32::from(!matches!(self.phase, Phase::Done));
        self.queue.len() as u32 + current
    }
}
