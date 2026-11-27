use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use ulid::Ulid;

use jiff::Timestamp;

/// A typed id: an `Id<Card>` can never be passed where an `Id<Review>` is
/// expected, though both are the same 16 bytes underneath.
///
/// A ULID: sortable by creation time, and unique without coordination, so
/// any device can mint one.
pub struct Id<T> {
    ulid: Ulid,
    // `fn() -> T` keeps `Id<T>` Send, Sync and Copy whatever `T` is.
    kind: PhantomData<fn() -> T>,
}

/// Identifies a card.
pub type CardId = Id<Card>;
/// Identifies one review. Writing the same review twice is a no-op.
pub type ReviewId = Id<Review>;

impl<T> Id<T> {
    pub fn new(at: Timestamp, entropy: u128) -> Self {
        Self::from_ulid(Ulid::from_parts(at.as_millisecond().max(0) as u64, entropy))
    }

    /// 16 bytes, big-endian, so byte order matches creation order.
    pub fn to_bytes(self) -> [u8; 16] {
        self.ulid.to_bytes()
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self::from_ulid(Ulid::from_bytes(bytes))
    }

    const fn from_ulid(ulid: Ulid) -> Self {
        Self {
            ulid,
            kind: PhantomData,
        }
    }
}

// Written by hand because `#[derive]` would demand `T: Clone`, `T: Eq`, and
// so on, which `Card` and `Review` need not be.
impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ulid == other.ulid
    }
}

impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ulid.cmp(&other.ulid)
    }
}

impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.ulid.hash(state);
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.ulid)
    }
}

/// A two-sided card. Notes, reverse cards and cloze can come later: reviews
/// only reference [`CardId`], so adding them does not touch the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: CardId,
    pub front: String,
    pub back: String,
}

#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

impl Rating {
    pub const fn as_u8(self) -> u8 {
        match self {
            Rating::Again => 1,
            Rating::Hard => 2,
            Rating::Good => 3,
            Rating::Easy => 4,
        }
    }

    pub const fn from_u8(n: u8) -> Option<Self> {
        match n {
            1 => Some(Rating::Again),
            2 => Some(Rating::Hard),
            3 => Some(Rating::Good),
            4 => Some(Rating::Easy),
            _ => None,
        }
    }
}

/// One review: an immutable fact in the log. A card's schedule is derived
/// from its reviews, never stored as the truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub id: ReviewId,
    pub card: CardId,
    pub at: Timestamp,
    pub rating: Rating,
    pub took_ms: u32,
}
