//! Almond's core.
//!
//! Everything in this crate is pure: no clock, no randomness, no I/O. Time and
//! entropy arrive as inputs ([`Env`]), and every side effect leaves as a value
//! ([`Cmd`]) for a runtime to perform. That keeps `update` deterministic, so the
//! same messages always produce the same model, the same view, and the same
//! commands, in tests and on device alike.

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

mod app;
mod card;
mod schedule;
mod session;
mod time;

pub use app::{Cmd, DbCmd, Env, Event, Haptic, Model, Msg, PlatformCmd, update};
pub use card::{Card, CardId, Id, Rating, Review, ReviewId};
pub use schedule::{CardState, MemoryState, Preview, ScheduleError, Scheduler};
pub use session::Due;
pub use time::Timestamp;
pub use view::{CardView, Intervals, ViewModel, view};

mod view;
