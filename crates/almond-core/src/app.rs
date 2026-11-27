use std::mem;

use crate::card::{Card, CardId, Rating, Review, ReviewId};
use crate::schedule::{CardState, Scheduler};
use crate::session::{Due, Phase, Session, Undo};
use crate::time::{end_of_day, millis_between};
use jiff::{Timestamp, tz::TimeZone};

/// How many cards one session loads.
const SESSION_LIMIT: u32 = 500;

/// What the user did. This is the only input the shell can send.
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The app launched or came to the foreground.
    Opened,
    AddCard {
        front: String,
        back: String,
    },
    StartReview,
    Reveal,
    Rate {
        rating: Rating,
    },
    Undo,
    /// Leave a session, finished or not.
    Close,
}

/// Everything `update` reacts to: user events and results of commands.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Event(Event),
    DueLoaded(Vec<Due>),
    /// Loading the due cards failed; the message is for display.
    LoadFailed(String),
}

impl From<Event> for Msg {
    fn from(event: Event) -> Self {
        Msg::Event(event)
    }
}

/// The world as `update` sees it. The runtime fills it in; tests pick values.
#[derive(Debug, Clone, PartialEq)]
pub struct Env {
    pub now: Timestamp,
    /// The user's time zone, for where their study day starts and ends.
    pub tz: TimeZone,
    /// Random bits for minting ids.
    pub entropy: u128,
}

/// Side effects, as data. The runtime performs them; tests assert on them.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// The view changed; the shell should read it again.
    Render,
    Db(DbCmd),
    Platform(PlatformCmd),
}

/// Storage work. Every write is idempotent, keyed by id.
#[derive(Debug, Clone, PartialEq)]
pub enum DbCmd {
    /// Cards due before `due_by`. Answers with [`Msg::DueLoaded`].
    LoadDue {
        due_by: Timestamp,
        limit: u32,
    },
    InsertCard(Card),
    /// Appends `review` to the log and caches `state` for its card.
    RecordReview {
        review: Review,
        state: CardState,
    },
    /// Marks `review` undone and puts back the card's previous cached state.
    UndoReview {
        review: ReviewId,
        card: CardId,
        restore: Option<CardState>,
    },
}

/// Work only the platform can do. Swift handles these and nothing else.
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformCmd {
    Haptic { kind: Haptic },
}

#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Haptic {
    Reveal,
    Rate,
    Finished,
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub(crate) scheduler: Scheduler,
    pub(crate) screen: Screen,
}

// One Screen exists per app, so the size gap between variants costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) enum Screen {
    #[default]
    Loading,
    Home {
        due: Vec<Due>,
    },
    Review(Session),
    /// Storage failed. `Opened` retries.
    Failed {
        message: String,
    },
}

impl Model {
    pub fn new(scheduler: Scheduler) -> Self {
        Self {
            scheduler,
            screen: Screen::Loading,
        }
    }
}

/// The whole app's behavior. Pure: given the same model, message and env, it
/// always makes the same changes and returns the same commands.
pub fn update(model: &mut Model, msg: Msg, env: &Env) -> Vec<Cmd> {
    let now = env.now;
    match (msg, &mut model.screen) {
        // Coming back to the foreground mid-review keeps the session.
        (Msg::Event(Event::Opened), Screen::Review(_)) => vec![],

        (Msg::Event(Event::Opened | Event::Close), _) => {
            model.screen = Screen::Loading;
            vec![
                Cmd::Db(DbCmd::LoadDue {
                    due_by: end_of_day(now, &env.tz),
                    limit: SESSION_LIMIT,
                }),
                Cmd::Render,
            ]
        }

        // A newer load (say, after a card was added mid-load) replaces the list.
        (Msg::DueLoaded(due), Screen::Loading | Screen::Home { .. }) => {
            model.screen = Screen::Home { due };
            vec![Cmd::Render]
        }

        (Msg::LoadFailed(message), Screen::Loading) => {
            model.screen = Screen::Failed { message };
            vec![Cmd::Render]
        }

        (Msg::Event(Event::AddCard { front, back }), screen) => {
            let (front, back) = (front.trim(), back.trim());
            if front.is_empty() || back.is_empty() {
                return vec![];
            }
            let card = Card {
                id: CardId::new(now, env.entropy),
                front: front.to_owned(),
                back: back.to_owned(),
            };
            let due = Due {
                card: card.clone(),
                state: None,
            };
            let mut cmds = vec![Cmd::Db(DbCmd::InsertCard(card)), Cmd::Render];
            match screen {
                Screen::Home { due: list } => list.push(due),
                Screen::Review(session) => {
                    session.queue.push_back(due);
                    if matches!(session.phase, Phase::Done) {
                        session.show_next(now);
                    }
                }
                // The load in flight may miss the new card; queue another
                // behind the insert.
                Screen::Loading | Screen::Failed { .. } => cmds.push(Cmd::Db(DbCmd::LoadDue {
                    due_by: end_of_day(now, &env.tz),
                    limit: SESSION_LIMIT,
                })),
            }
            cmds
        }

        (Msg::Event(Event::StartReview), Screen::Home { due }) => {
            model.screen = Screen::Review(Session::start(mem::take(due), now));
            vec![Cmd::Render]
        }

        (Msg::Event(Event::Reveal), Screen::Review(session))
            if matches!(session.phase, Phase::Front { .. }) =>
        {
            reveal(session, &model.scheduler, now)
        }

        (Msg::Event(Event::Rate { rating }), Screen::Review(session))
            if matches!(session.phase, Phase::Back { .. }) =>
        {
            rate(session, &model.scheduler, rating, env)
        }

        (Msg::Event(Event::Undo), Screen::Review(session)) => undo(session, now),

        // Anything else is stale or out of order (a double tap, a late
        // result): ignore it rather than corrupt the flow.
        _ => vec![],
    }
}

fn reveal(session: &mut Session, scheduler: &Scheduler, now: Timestamp) -> Vec<Cmd> {
    let Phase::Front { current, shown_at } = mem::replace(&mut session.phase, Phase::Done) else {
        unreachable!("update only calls reveal from Front")
    };
    let preview = scheduler.preview(current.state.as_ref(), now);
    session.phase = Phase::Back {
        current,
        shown_at,
        preview,
    };
    vec![
        Cmd::Render,
        Cmd::Platform(PlatformCmd::Haptic {
            kind: Haptic::Reveal,
        }),
    ]
}

fn rate(session: &mut Session, scheduler: &Scheduler, rating: Rating, env: &Env) -> Vec<Cmd> {
    let now = env.now;
    let Phase::Back {
        current,
        shown_at,
        preview,
    } = mem::replace(&mut session.phase, Phase::Done)
    else {
        unreachable!("update only calls rate from Back")
    };
    let review = Review {
        id: ReviewId::new(now, env.entropy),
        card: current.card.id,
        at: now,
        rating,
        took_ms: millis_between(now, shown_at),
    };
    // Computed from the review itself, not the preview, so the cache always
    // equals a fold over the log.
    let state = scheduler.advance(current.state.as_ref(), rating, now);
    if rating == Rating::Again {
        session.queue.push_back(Due {
            card: current.card.clone(),
            state: Some(state),
        });
    }
    session.undo = Some(Undo {
        review: review.id,
        current,
        preview,
    });
    session.reviewed += 1;
    session.show_next(now);
    let haptic = if matches!(session.phase, Phase::Done) {
        Haptic::Finished
    } else {
        Haptic::Rate
    };
    vec![
        Cmd::Db(DbCmd::RecordReview { review, state }),
        Cmd::Render,
        Cmd::Platform(PlatformCmd::Haptic { kind: haptic }),
    ]
}

fn undo(session: &mut Session, now: Timestamp) -> Vec<Cmd> {
    let Some(undo) = session.undo.take() else {
        return vec![];
    };
    let card = undo.current.card.id;
    // Drop the requeued copy from an Again, and put whatever is on screen
    // back at the front.
    session.queue.retain(|d| d.card.id != card);
    if let Phase::Front { current, .. } | Phase::Back { current, .. } =
        mem::replace(&mut session.phase, Phase::Done)
        && current.card.id != card
    {
        session.queue.push_front(current);
    }
    let restore = undo.current.state;
    session.reviewed -= 1;
    session.phase = Phase::Back {
        current: undo.current,
        // Time spent on other cards in between is not time spent on this one.
        shown_at: now,
        preview: undo.preview,
    };
    vec![
        Cmd::Db(DbCmd::UndoReview {
            review: undo.review,
            card,
            restore,
        }),
        Cmd::Render,
    ]
}
