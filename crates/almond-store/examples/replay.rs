//! Measures the costs the design doc estimated: rating one card, loading the
//! due queue, and refolding the whole log after re-optimizing.
//!
//! Run with `cargo run --release -p almond-store --example replay`.

use std::time::Instant;

use almond_core::{Card, CardId, DbCmd, Rating, Review, ReviewId, Scheduler, Timestamp};
use almond_store::Store;

const CARDS: u128 = 1_000;
const REVIEWS_PER_CARD: i64 = 365;
const DAY: i64 = 86_400_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let s = Scheduler::default();
    let path = std::env::temp_dir().join("almond-replay.db");
    let _ = std::fs::remove_file(&path);
    let mut store = Store::open(&path)?;
    let start = Timestamp::from_millisecond(1_700_000_000_000)?;
    let ratings = [
        Rating::Good,
        Rating::Good,
        Rating::Hard,
        Rating::Good,
        Rating::Again,
        Rating::Easy,
    ];

    let t = Instant::now();
    let mut n = 0u128;
    let mut rate_ns = 0u128;
    for c in 0..CARDS {
        let id = CardId::new(start, c);
        store.apply(DbCmd::InsertCard(Card {
            id,
            front: format!("front {c}"),
            back: "back".into(),
        }))?;
        let mut state = None;
        for i in 0..REVIEWS_PER_CARD {
            n += 1;
            let at = Timestamp::from_millisecond(start.as_millisecond() + i * DAY / 3)?;
            let rating = ratings[(n % ratings.len() as u128) as usize];
            let step = Instant::now();
            let next = s.advance(state.as_ref(), rating, at);
            rate_ns += step.elapsed().as_nanos();
            state = Some(next);
            let review = Review {
                id: ReviewId::new(at, n),
                card: id,
                at,
                rating,
                took_ms: 1_000,
            };
            store.apply(DbCmd::RecordReview {
                review,
                state: next,
            })?;
        }
    }
    println!(
        "wrote {n} reviews in {:.1?} (one transaction each)",
        t.elapsed()
    );
    println!(
        "one FSRS step: {:.2} µs on average",
        rate_ns as f64 / n as f64 / 1_000.0
    );

    let t = Instant::now();
    let due = store.due(
        Timestamp::from_millisecond(start.as_millisecond() + 400 * DAY)?,
        500,
    )?;
    println!("load {} due cards: {:.2?}", due.len(), t.elapsed());

    let t = Instant::now();
    let rebuilt = store.rebuild_states(&s)?;
    println!(
        "refold {n} reviews into {rebuilt} card states: {:.2?}",
        t.elapsed()
    );

    let size = std::fs::metadata(&path)?.len();
    println!("database size: {:.1} MB", size as f64 / 1e6);
    Ok(())
}
