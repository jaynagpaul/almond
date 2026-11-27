use almond_core::{Card, CardId, DbCmd, Msg, Rating, Review, ReviewId, Scheduler, Timestamp};
use almond_store::Store;

const DAY: i64 = 86_400_000;

fn at(day: i64) -> Timestamp {
    Timestamp::from_millisecond(1_800_000_000_000 + day * DAY).unwrap()
}

fn new_card(store: &mut Store, n: u128, front: &str) -> CardId {
    let card = Card {
        id: CardId::new(at(0), n),
        front: front.into(),
        back: "back".into(),
    };
    let id = card.id;
    store.apply(DbCmd::InsertCard(card)).unwrap();
    id
}

/// Reviews `card` the way the runtime does: one FSRS step from the cached state.
fn review(
    store: &mut Store,
    s: &Scheduler,
    card: CardId,
    day: i64,
    rating: Rating,
    n: u128,
) -> Review {
    let prev = store.state(card).unwrap();
    let review = Review {
        id: ReviewId::new(at(day), n),
        card,
        at: at(day),
        rating,
        took_ms: 1_000,
    };
    let state = s.advance(prev.as_ref(), rating, at(day));
    store
        .apply(DbCmd::RecordReview {
            review: review.clone(),
            state,
        })
        .unwrap();
    review
}

fn due_fronts(store: &mut Store, day: i64) -> Vec<String> {
    match store
        .apply(DbCmd::LoadDue {
            due_by: at(day),
            limit: 100,
        })
        .unwrap()
    {
        Some(Msg::DueLoaded(due)) => due.into_iter().map(|d| d.card.front).collect(),
        other => panic!("expected DueLoaded, got {other:?}"),
    }
}

#[test]
fn new_cards_are_due_in_the_order_they_were_added() {
    let mut store = Store::open_in_memory().unwrap();
    new_card(&mut store, 2, "second");
    new_card(&mut store, 1, "first");
    assert_eq!(due_fronts(&mut store, 0), ["first", "second"]);
}

#[test]
fn a_reviewed_card_leaves_the_queue_until_it_is_due() {
    let s = Scheduler::default();
    let mut store = Store::open_in_memory().unwrap();
    let card = new_card(&mut store, 1, "q");
    review(&mut store, &s, card, 0, Rating::Good, 1);

    assert!(due_fronts(&mut store, 0).is_empty());
    let due = store.state(card).unwrap().unwrap().due;
    let due_day = (due.as_millisecond() - at(0).as_millisecond()) / DAY + 1;
    assert_eq!(due_fronts(&mut store, due_day), ["q"]);
}

#[test]
fn recording_the_same_review_twice_changes_nothing() {
    let s = Scheduler::default();
    let mut store = Store::open_in_memory().unwrap();
    let card = new_card(&mut store, 1, "q");
    let r = review(&mut store, &s, card, 0, Rating::Good, 1);
    let state = store.state(card).unwrap().unwrap();

    // A retry with a different (wrong) state must not overwrite the cache.
    let bogus = s.advance(None, Rating::Again, at(9));
    store
        .apply(DbCmd::RecordReview {
            review: r,
            state: bogus,
        })
        .unwrap();
    assert_eq!(store.reviews(card).unwrap().len(), 1);
    assert_eq!(store.state(card).unwrap(), Some(state));
}

#[test]
fn undo_restores_the_previous_state_and_drops_the_review_from_the_fold() {
    let s = Scheduler::default();
    let mut store = Store::open_in_memory().unwrap();
    let card = new_card(&mut store, 1, "q");
    review(&mut store, &s, card, 0, Rating::Good, 1);
    let before = store.state(card).unwrap();
    let r = review(&mut store, &s, card, 4, Rating::Again, 2);

    for _ in 0..2 {
        store
            .apply(DbCmd::UndoReview {
                review: r.id,
                card,
                restore: before,
            })
            .unwrap();
    }
    assert_eq!(store.state(card).unwrap(), before);
    assert_eq!(store.reviews(card).unwrap().len(), 1);
}

#[test]
fn undoing_a_first_review_makes_the_card_new_again() {
    let s = Scheduler::default();
    let mut store = Store::open_in_memory().unwrap();
    let card = new_card(&mut store, 1, "q");
    let r = review(&mut store, &s, card, 0, Rating::Good, 1);
    store
        .apply(DbCmd::UndoReview {
            review: r.id,
            card,
            restore: None,
        })
        .unwrap();
    assert_eq!(store.state(card).unwrap(), None);
    assert_eq!(due_fronts(&mut store, 0), ["q"]);
}

#[test]
fn rebuilding_from_the_log_reproduces_the_cache_exactly() {
    let s = Scheduler::default();
    let mut store = Store::open_in_memory().unwrap();
    let ratings = [
        Rating::Good,
        Rating::Again,
        Rating::Hard,
        Rating::Easy,
        Rating::Good,
    ];
    let mut n = 0;
    let cards: Vec<_> = (0..20)
        .map(|i| new_card(&mut store, 1_000 + i, "q"))
        .collect();
    for (i, card) in cards.iter().enumerate() {
        for (day, rating) in ratings.iter().cycle().skip(i).take(5).enumerate() {
            n += 1;
            review(&mut store, &s, *card, day as i64 * 3, *rating, n);
        }
    }
    let cached: Vec<_> = cards.iter().map(|c| store.state(*c).unwrap()).collect();

    assert_eq!(store.rebuild_states(&s).unwrap(), 20);
    let rebuilt: Vec<_> = cards.iter().map(|c| store.state(*c).unwrap()).collect();
    assert_eq!(cached, rebuilt);
}

#[test]
fn reopening_a_database_keeps_its_data() {
    let dir = std::env::temp_dir().join(format!("almond-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("almond.db");
    {
        let mut store = Store::open(&path).unwrap();
        new_card(&mut store, 1, "kept");
    }
    let mut store = Store::open(&path).unwrap();
    assert_eq!(due_fronts(&mut store, 0), ["kept"]);
    std::fs::remove_dir_all(dir).unwrap();
}
