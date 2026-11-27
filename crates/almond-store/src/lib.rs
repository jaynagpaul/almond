//! SQLite storage for Almond.
//!
//! The `reviews` table is the source of truth and is append-only. The
//! `card_state` table caches each card's schedule so reads never replay the
//! log; [`Store::rebuild_states`] can recreate it from the log at any time.
//! Every write is keyed by id, so running a command twice changes nothing.

use std::path::Path;

use almond_core::{
    Card, CardId, CardState, DbCmd, Due, MemoryState, Msg, Rating, Review, ReviewId, Scheduler,
    Timestamp,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub use rusqlite::Error;

const SCHEMA: &str = "
-- Ids are 16-byte ULIDs, so they sort by creation time.
CREATE TABLE cards (
    id    BLOB PRIMARY KEY,
    front TEXT NOT NULL,
    back  TEXT NOT NULL
) STRICT;

CREATE TABLE reviews (
    id      BLOB PRIMARY KEY,
    card    BLOB NOT NULL REFERENCES cards(id),
    at      INTEGER NOT NULL,
    rating  INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 4),
    took_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
CREATE INDEX reviews_by_card ON reviews(card, at, id);

-- Undo appends here instead of deleting from the log.
CREATE TABLE undone_reviews (
    review BLOB PRIMARY KEY REFERENCES reviews(id)
) STRICT, WITHOUT ROWID;

-- A cache of folding each card's live reviews. Rebuildable from the log.
CREATE TABLE card_state (
    card           BLOB PRIMARY KEY REFERENCES cards(id),
    stability      REAL NOT NULL,
    difficulty     REAL NOT NULL,
    stability_fast REAL NOT NULL,
    last_review    INTEGER NOT NULL,
    due            INTEGER NOT NULL
) STRICT;
CREATE INDEX card_state_by_due ON card_state(due);
";

pub struct Store {
    conn: Connection,
}

impl Store {
    /// # Errors
    /// If the database cannot be opened or migrated.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::init(Connection::open(path)?)
    }

    /// # Errors
    /// If the in-memory database cannot be created.
    pub fn open_in_memory() -> Result<Self, Error> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, Error> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 0 {
            conn.execute_batch(&format!("BEGIN; {SCHEMA} PRAGMA user_version = 1; COMMIT;"))?;
        }
        Ok(Self { conn })
    }

    /// Performs one command. Returns the message to feed back to `update`, if any.
    ///
    /// # Errors
    /// On any SQLite failure.
    pub fn apply(&mut self, cmd: DbCmd) -> Result<Option<Msg>, Error> {
        match cmd {
            DbCmd::LoadDue {
                now,
                day_end,
                limit,
            } => Ok(Some(Msg::DueLoaded(self.due(now, day_end, limit)?))),
            DbCmd::InsertCard(card) => {
                self.conn.execute(
                    "INSERT OR IGNORE INTO cards (id, front, back) VALUES (?1, ?2, ?3)",
                    params![card.id.to_bytes(), card.front, card.back],
                )?;
                Ok(None)
            }
            DbCmd::RecordReview { review, state } => {
                let tx = self.conn.transaction()?;
                let inserted = tx.execute(
                    "INSERT OR IGNORE INTO reviews (id, card, at, rating, took_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        review.id.to_bytes(),
                        review.card.to_bytes(),
                        review.at.as_millisecond(),
                        review.rating.as_u8(),
                        review.took_ms
                    ],
                )?;
                if inserted == 1 {
                    put_state(&tx, review.card, &state)?;
                }
                tx.commit()?;
                Ok(None)
            }
            DbCmd::UndoReview {
                review,
                card,
                restore,
            } => {
                let tx = self.conn.transaction()?;
                let inserted = tx.execute(
                    "INSERT OR IGNORE INTO undone_reviews (review) VALUES (?1)",
                    [review.to_bytes()],
                )?;
                if inserted == 1 {
                    match restore {
                        Some(state) => put_state(&tx, card, &state)?,
                        None => {
                            tx.execute(
                                "DELETE FROM card_state WHERE card = ?1",
                                [card.to_bytes()],
                            )?;
                        }
                    }
                }
                tx.commit()?;
                Ok(None)
            }
        }
    }

    /// Cards due by `now`, and cards with intervals of a day or more due before
    /// `day_end`; most overdue first, then new cards oldest first.
    ///
    /// # Errors
    /// On any SQLite failure.
    pub fn due(&self, now: Timestamp, day_end: Timestamp, limit: u32) -> Result<Vec<Due>, Error> {
        let mut out = Vec::new();
        let mut reviewed = self.conn.prepare_cached(
            "SELECT c.id, c.front, c.back,
                    s.stability, s.difficulty, s.stability_fast, s.last_review, s.due
             FROM card_state s JOIN cards c ON c.id = s.card
             WHERE s.due <= ?1 OR (s.due < ?2 AND s.due - s.last_review >= 86400000)
             ORDER BY s.due LIMIT ?3",
        )?;
        for due in reviewed.query_map(
            params![now.as_millisecond(), day_end.as_millisecond(), limit],
            |r| {
                Ok(Due {
                    card: card(r)?,
                    state: Some(CardState {
                        memory: MemoryState {
                            stability: r.get(3)?,
                            difficulty: r.get(4)?,
                            stability_fast: r.get(5)?,
                        },
                        last_review: ts(r, 6)?,
                        due: ts(r, 7)?,
                    }),
                })
            },
        )? {
            out.push(due?);
        }
        let room = limit.saturating_sub(out.len() as u32);
        let mut new = self.conn.prepare_cached(
            "SELECT c.id, c.front, c.back FROM cards c
             LEFT JOIN card_state s ON s.card = c.id
             WHERE s.card IS NULL ORDER BY c.id LIMIT ?1",
        )?;
        for due in new.query_map([room], |r| {
            Ok(Due {
                card: card(r)?,
                state: None,
            })
        })? {
            out.push(due?);
        }
        Ok(out)
    }

    /// A card's live reviews (not undone), oldest first.
    ///
    /// # Errors
    /// On any SQLite failure.
    pub fn reviews(&self, card: CardId) -> Result<Vec<Review>, Error> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, card, at, rating, took_ms FROM reviews
             WHERE card = ?1 AND id NOT IN (SELECT review FROM undone_reviews)
             ORDER BY at, id",
        )?;
        stmt.query_map([card.to_bytes()], review)?.collect()
    }

    /// The cached state for one card, if it has been reviewed.
    ///
    /// # Errors
    /// On any SQLite failure.
    pub fn state(&self, card: CardId) -> Result<Option<CardState>, Error> {
        self.conn
            .query_row(
                "SELECT stability, difficulty, stability_fast, last_review, due
                 FROM card_state WHERE card = ?1",
                [card.to_bytes()],
                |r| {
                    Ok(CardState {
                        memory: MemoryState {
                            stability: r.get(0)?,
                            difficulty: r.get(1)?,
                            stability_fast: r.get(2)?,
                        },
                        last_review: ts(r, 3)?,
                        due: ts(r, 4)?,
                    })
                },
            )
            .optional()
    }

    /// Recreates every card's cached state by folding the log, for example
    /// after new FSRS parameters. Returns how many cards have a state.
    ///
    /// # Errors
    /// On any SQLite failure.
    pub fn rebuild_states(&mut self, scheduler: &Scheduler) -> Result<usize, Error> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM card_state", [])?;
        let mut count = 0;
        {
            let mut stmt = tx.prepare(
                "SELECT id, card, at, rating, took_ms FROM reviews
                 WHERE id NOT IN (SELECT review FROM undone_reviews)
                 ORDER BY card, at, id",
            )?;
            let mut rows = stmt.query_map([], review)?.peekable();
            let mut batch = Vec::new();
            while let Some(r) = rows.next() {
                let r = r?;
                let card = r.card;
                batch.push(r);
                let last_of_card = !matches!(rows.peek(), Some(Ok(next)) if next.card == card);
                if last_of_card {
                    if let Some(state) = scheduler.fold(&batch) {
                        put_state(&tx, card, &state)?;
                        count += 1;
                    }
                    batch.clear();
                }
            }
        }
        tx.commit()?;
        Ok(count)
    }
}

fn put_state(conn: &Connection, card: CardId, s: &CardState) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO card_state (card, stability, difficulty, stability_fast, last_review, due)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (card) DO UPDATE SET
           stability = excluded.stability, difficulty = excluded.difficulty,
           stability_fast = excluded.stability_fast,
           last_review = excluded.last_review, due = excluded.due",
        params![
            card.to_bytes(),
            s.memory.stability,
            s.memory.difficulty,
            s.memory.stability_fast,
            s.last_review.as_millisecond(),
            s.due.as_millisecond()
        ],
    )?;
    Ok(())
}

fn id<T>(r: &Row, i: usize, from_bytes: fn([u8; 16]) -> T) -> Result<T, Error> {
    let bytes: [u8; 16] = r.get(i)?;
    Ok(from_bytes(bytes))
}

/// Times are stored as milliseconds since the Unix epoch, UTC.
fn ts(r: &Row, i: usize) -> Result<Timestamp, Error> {
    let ms: i64 = r.get(i)?;
    Timestamp::from_millisecond(ms).map_err(|e| {
        Error::FromSqlConversionFailure(i, rusqlite::types::Type::Integer, Box::new(e))
    })
}

fn card(r: &Row) -> Result<Card, Error> {
    Ok(Card {
        id: id(r, 0, CardId::from_bytes)?,
        front: r.get(1)?,
        back: r.get(2)?,
    })
}

fn review(r: &Row) -> Result<Review, Error> {
    let rating: u8 = r.get(3)?;
    Ok(Review {
        id: id(r, 0, ReviewId::from_bytes)?,
        card: id(r, 1, CardId::from_bytes)?,
        at: ts(r, 2)?,
        rating: Rating::from_u8(rating)
            .ok_or(Error::IntegralValueOutOfRange(3, i64::from(rating)))?,
        took_ms: r.get(4)?,
    })
}
