use crate::app::{Model, Screen};
use crate::schedule::Preview;
use crate::session::Phase;

/// Everything the shell draws. Plain data, no behavior.
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq)]
pub enum ViewModel {
    Loading,
    Home {
        due: u32,
    },
    Review {
        card: CardView,
    },
    Done {
        reviewed: u32,
        can_undo: bool,
    },
    /// Storage failed. Sending `Opened` retries.
    Failed {
        message: String,
    },
}

#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    pub front: String,
    /// `None` until revealed.
    pub back: Option<String>,
    /// Labels for the rating buttons, once revealed.
    pub intervals: Option<Intervals>,
    /// Cards left, including this one.
    pub remaining: u32,
    pub can_undo: bool,
}

#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intervals {
    pub again: String,
    pub hard: String,
    pub good: String,
    pub easy: String,
}

pub fn view(model: &Model) -> ViewModel {
    match &model.screen {
        Screen::Loading => ViewModel::Loading,
        Screen::Failed { message } => ViewModel::Failed {
            message: message.clone(),
        },
        Screen::Home { due } => ViewModel::Home {
            due: due.len() as u32,
        },
        Screen::Review(session) => {
            let remaining = session.remaining();
            let can_undo = session.undo.is_some();
            match &session.phase {
                Phase::Front { current, .. } => ViewModel::Review {
                    card: CardView {
                        front: current.card.front.clone(),
                        back: None,
                        intervals: None,
                        remaining,
                        can_undo,
                    },
                },
                Phase::Back {
                    current, preview, ..
                } => ViewModel::Review {
                    card: CardView {
                        front: current.card.front.clone(),
                        back: Some(current.card.back.clone()),
                        intervals: Some(intervals(preview)),
                        remaining,
                        can_undo,
                    },
                },
                Phase::Done => ViewModel::Done {
                    reviewed: session.reviewed,
                    can_undo,
                },
            }
        }
    }
}

fn intervals(p: &Preview) -> Intervals {
    Intervals {
        again: label(p.again),
        hard: label(p.hard),
        good: label(p.good),
        easy: label(p.easy),
    }
}

/// A short human label for an interval in days: `10m`, `5h`, `3d`, `2.5mo`, `1.2y`.
fn label(days: f32) -> String {
    let one_decimal = |x: f32| {
        let s = format!("{x:.1}");
        s.strip_suffix(".0").map_or(s.clone(), str::to_owned)
    };
    if days < 1.0 / 24.0 {
        format!("{}m", (days * 1440.0).round().max(1.0))
    } else if days < 1.0 {
        format!("{}h", (days * 24.0).round())
    } else if days < 30.0 {
        format!("{}d", days.round())
    } else if days < 365.0 {
        format!("{}mo", one_decimal(days / 30.44))
    } else {
        format!("{}y", one_decimal(days / 365.25))
    }
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn labels_read_like_a_person_would_say_them() {
        assert_eq!(label(0.0), "1m");
        assert_eq!(label(10.0 / 1440.0), "10m");
        assert_eq!(label(0.5), "12h");
        assert_eq!(label(3.4), "3d");
        assert_eq!(label(61.0), "2mo");
        assert_eq!(label(76.0), "2.5mo");
        assert_eq!(label(400.0), "1.1y");
    }
}
