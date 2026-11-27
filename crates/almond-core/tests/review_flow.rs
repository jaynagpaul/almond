//! Behavior tests: drive the app through `update` and check what a user
//! would see (`view`) and what the runtime is asked to do (`Cmd`).

use almond_core::{
    Card, CardId, Cmd, DbCmd, Due, Env, Event, Haptic, Model, Msg, PlatformCmd, Rating, Scheduler,
    TimeZone, Timestamp, ViewModel, update, view,
};

const DAY: i64 = 86_400_000;

struct App {
    model: Model,
    now: Timestamp,
    entropy: u128,
}

impl App {
    fn with_cards(cards: &[(&str, &str)]) -> Self {
        let mut app = App {
            model: Model::default(),
            now: Timestamp::from_millisecond(1_800_000_000_000).unwrap(),
            entropy: 0,
        };
        app.send(Event::Opened);
        let due = cards
            .iter()
            .enumerate()
            .map(|(i, (front, back))| Due {
                card: Card {
                    id: CardId::new(app.now, i as u128),
                    front: (*front).into(),
                    back: (*back).into(),
                },
                state: None,
            })
            .collect();
        app.msg(Msg::DueLoaded(due));
        app
    }

    fn send(&mut self, event: Event) -> Vec<Cmd> {
        self.msg(event.into())
    }

    fn msg(&mut self, msg: Msg) -> Vec<Cmd> {
        self.entropy += 1;
        update(
            &mut self.model,
            msg,
            &Env {
                now: self.now,
                tz: TimeZone::UTC,
                entropy: self.entropy,
            },
        )
    }

    fn wait_ms(&mut self, ms: i64) {
        self.now = Timestamp::from_millisecond(self.now.as_millisecond() + ms).unwrap();
    }

    fn view(&self) -> ViewModel {
        view(&self.model)
    }

    fn card(&self) -> almond_core::CardView {
        match self.view() {
            ViewModel::Review { card } => card,
            other => panic!("expected a card on screen, got {other:?}"),
        }
    }
}

fn recorded(cmds: &[Cmd]) -> Vec<&almond_core::Review> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Db(DbCmd::RecordReview { review, .. }) => Some(review),
            _ => None,
        })
        .collect()
}

#[test]
fn opening_loads_cards_due_by_the_end_of_the_local_day_then_shows_home() {
    let mut model = Model::default();
    let env = Env {
        // 1:48 AM in Kolkata: the study day runs until 4 AM local.
        now: "2026-11-27T20:18:00Z".parse().unwrap(),
        tz: TimeZone::get("Asia/Kolkata").unwrap(),
        entropy: 0,
    };
    let cmds = update(&mut model, Event::Opened.into(), &env);
    assert_eq!(view(&model), ViewModel::Loading);
    assert!(cmds.contains(&Cmd::Db(DbCmd::LoadDue {
        due_by: "2026-11-27T22:30:00Z".parse().unwrap(),
        limit: 500
    })));

    update(&mut model, Msg::DueLoaded(vec![]), &env);
    assert_eq!(view(&model), ViewModel::Home { due: 0 });
}

#[test]
fn a_card_shows_its_front_then_its_back_with_interval_labels() {
    let mut app = App::with_cards(&[("bonjour", "hello")]);
    assert_eq!(app.view(), ViewModel::Home { due: 1 });

    app.send(Event::StartReview);
    let front = app.card();
    assert_eq!(front.front, "bonjour");
    assert_eq!(front.back, None);
    assert_eq!(front.intervals, None);
    assert_eq!(front.remaining, 1);

    let cmds = app.send(Event::Reveal);
    let back = app.card();
    assert_eq!(back.back.as_deref(), Some("hello"));
    let labels = back.intervals.expect("revealed cards show intervals");
    assert!(!labels.good.is_empty() && labels.good != labels.again);
    assert!(cmds.contains(&Cmd::Platform(PlatformCmd::Haptic {
        kind: Haptic::Reveal
    })));
}

#[test]
fn rating_before_reveal_does_nothing() {
    let mut app = App::with_cards(&[("q", "a")]);
    app.send(Event::StartReview);
    let before = app.view();
    let cmds = app.send(Event::Rate {
        rating: Rating::Good,
    });
    assert!(cmds.is_empty());
    assert_eq!(app.view(), before);
}

#[test]
fn rating_records_one_review_with_time_taken_and_moves_on() {
    let mut app = App::with_cards(&[("one", "1"), ("two", "2")]);
    app.send(Event::StartReview);
    app.wait_ms(1_500);
    app.send(Event::Reveal);
    app.wait_ms(500);
    let cmds = app.send(Event::Rate {
        rating: Rating::Good,
    });

    let reviews = recorded(&cmds);
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].rating, Rating::Good);
    assert_eq!(reviews[0].took_ms, 2_000);
    assert_eq!(app.card().front, "two");
    assert_eq!(app.card().remaining, 1);
}

#[test]
fn again_brings_the_card_back_later_in_the_session() {
    let mut app = App::with_cards(&[("one", "1"), ("two", "2")]);
    app.send(Event::StartReview);
    app.send(Event::Reveal);
    app.send(Event::Rate {
        rating: Rating::Again,
    });
    assert_eq!(app.card().front, "two");
    assert_eq!(app.card().remaining, 2);

    app.send(Event::Reveal);
    app.send(Event::Rate {
        rating: Rating::Good,
    });
    assert_eq!(app.card().front, "one");
}

#[test]
fn finishing_the_queue_shows_a_summary() {
    let mut app = App::with_cards(&[("one", "1")]);
    app.send(Event::StartReview);
    app.send(Event::Reveal);
    let cmds = app.send(Event::Rate {
        rating: Rating::Easy,
    });
    assert_eq!(
        app.view(),
        ViewModel::Done {
            reviewed: 1,
            can_undo: true
        }
    );
    assert!(cmds.contains(&Cmd::Platform(PlatformCmd::Haptic {
        kind: Haptic::Finished
    })));
}

#[test]
fn undo_puts_the_last_card_back_and_retracts_its_review() {
    let mut app = App::with_cards(&[("one", "1"), ("two", "2")]);
    app.send(Event::StartReview);
    app.send(Event::Reveal);
    let rated = app.send(Event::Rate {
        rating: Rating::Again,
    });
    let review_id = recorded(&rated)[0].id;

    let cmds = app.send(Event::Undo);
    let back = app.card();
    assert_eq!(back.front, "one");
    assert_eq!(back.back.as_deref(), Some("1"));
    assert_eq!(back.remaining, 2, "the requeued copy is gone");
    assert!(!back.can_undo);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Cmd::Db(DbCmd::UndoReview { review, restore: None, .. }) if *review == review_id
    )));

    app.send(Event::Rate {
        rating: Rating::Good,
    });
    assert_eq!(app.card().front, "two");
    assert_eq!(app.card().remaining, 1);
}

#[test]
fn blank_cards_are_not_added() {
    let mut app = App::with_cards(&[]);
    let cmds = app.send(Event::AddCard {
        front: "  ".into(),
        back: "x".into(),
    });
    assert!(cmds.is_empty());
    assert_eq!(app.view(), ViewModel::Home { due: 0 });

    let cmds = app.send(Event::AddCard {
        front: " chat ".into(),
        back: "cat".into(),
    });
    assert!(matches!(&cmds[0], Cmd::Db(DbCmd::InsertCard(c)) if c.front == "chat"));
    assert_eq!(app.view(), ViewModel::Home { due: 1 });
}

#[test]
fn the_cached_state_always_equals_a_fold_over_the_log() {
    let scheduler = Scheduler::default();
    let mut app = App::with_cards(&[("q", "a")]);
    let mut log = Vec::new();
    let mut last_state = None;
    for rating in [Rating::Good, Rating::Again, Rating::Hard, Rating::Easy] {
        app.send(Event::StartReview);
        app.send(Event::Reveal);
        let cmds = app.send(Event::Rate { rating });
        for c in cmds {
            if let Cmd::Db(DbCmd::RecordReview { review, state }) = c {
                log.push(review);
                last_state = Some(state);
            }
        }
        // Come back three days later with the card due again.
        app.send(Event::Close);
        app.wait_ms(3 * DAY);
        let card = Card {
            id: log[0].card,
            front: "q".into(),
            back: "a".into(),
        };
        app.msg(Msg::DueLoaded(vec![Due {
            card,
            state: last_state,
        }]));
    }
    assert_eq!(log.len(), 4);
    assert_eq!(scheduler.fold(&log), last_state);
}

#[test]
fn scheduling_is_deterministic() {
    let s = Scheduler::default();
    let at = Timestamp::UNIX_EPOCH;
    let a = s.advance(None, Rating::Good, at);
    let b = s.advance(None, Rating::Good, at);
    assert_eq!(a, b);
    assert!(a.due > at);
    assert!(Scheduler::new(&[], 0.5).is_err());
}

#[test]
fn revealing_twice_keeps_the_back_showing() {
    let mut app = App::with_cards(&[("q", "a")]);
    app.send(Event::StartReview);
    app.send(Event::Reveal);
    let shown = app.view();
    assert!(app.send(Event::Reveal).is_empty());
    assert_eq!(app.view(), shown);
}

#[test]
fn undo_after_revealing_the_next_card_keeps_that_card_in_the_session() {
    let mut app = App::with_cards(&[("one", "1"), ("two", "2")]);
    app.send(Event::StartReview);
    app.send(Event::Reveal);
    app.send(Event::Rate {
        rating: Rating::Good,
    });
    app.send(Event::Reveal); // "two" is now on its back
    app.send(Event::Undo);
    assert_eq!(app.card().front, "one");
    assert_eq!(app.card().remaining, 2);

    app.send(Event::Rate {
        rating: Rating::Good,
    });
    assert_eq!(app.card().front, "two");
}

#[test]
fn coming_back_to_the_foreground_keeps_the_session() {
    let mut app = App::with_cards(&[("one", "1"), ("two", "2")]);
    app.send(Event::StartReview);
    app.send(Event::Reveal);
    let shown = app.view();
    assert!(app.send(Event::Opened).is_empty());
    assert_eq!(app.view(), shown);
}

#[test]
fn rerating_after_undo_times_only_the_second_look() {
    let mut app = App::with_cards(&[("one", "1"), ("two", "2")]);
    app.send(Event::StartReview);
    app.send(Event::Reveal);
    app.wait_ms(2_000);
    app.send(Event::Rate {
        rating: Rating::Good,
    });
    app.wait_ms(60_000);
    app.send(Event::Undo);
    app.wait_ms(1_000);
    let cmds = app.send(Event::Rate {
        rating: Rating::Good,
    });
    assert_eq!(recorded(&cmds)[0].took_ms, 1_000);
}

#[test]
fn a_card_added_on_the_done_screen_is_shown_next() {
    let mut app = App::with_cards(&[("one", "1")]);
    app.send(Event::StartReview);
    app.send(Event::Reveal);
    app.send(Event::Rate {
        rating: Rating::Good,
    });
    app.send(Event::AddCard {
        front: "new".into(),
        back: "card".into(),
    });
    assert_eq!(app.card().front, "new");
}

#[test]
fn a_failed_load_shows_an_error_and_opening_again_retries() {
    let mut model = Model::default();
    let env = Env {
        now: Timestamp::UNIX_EPOCH,
        tz: TimeZone::UTC,
        entropy: 0,
    };
    update(&mut model, Event::Opened.into(), &env);
    update(&mut model, Msg::LoadFailed("disk full".into()), &env);
    assert_eq!(
        view(&model),
        ViewModel::Failed {
            message: "disk full".into()
        }
    );
    let cmds = update(&mut model, Event::Opened.into(), &env);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::Db(DbCmd::LoadDue { .. })))
    );
    assert_eq!(view(&model), ViewModel::Loading);
}
