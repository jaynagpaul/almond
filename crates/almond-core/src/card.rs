use std::fmt;
use std::str::FromStr;

use ulid::Ulid;

use crate::time::Timestamp;

/// A string that is not a valid id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidId(pub String);

impl fmt::Display for InvalidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a valid id: {:?}", self.0)
    }
}

impl std::error::Error for InvalidId {}

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// A ULID: sortable by creation time, and unique without coordination,
        /// so any device can mint one.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Ulid);

        impl $name {
            pub fn new(at: Timestamp, entropy: u128) -> Self {
                Self(Ulid::from_parts(at.as_millis() as u64, entropy))
            }
        }

        impl $name {
            /// 16 bytes, big-endian, so byte order matches creation order.
            pub fn to_bytes(self) -> [u8; 16] {
                self.0.to_bytes()
            }

            pub fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(Ulid::from_bytes(bytes))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ulid::from_string(s).map(Self).map_err(|_| InvalidId(s.to_owned()))
            }
        }
    };
}

id!(
    /// Identifies a card.
    CardId
);
id!(
    /// Identifies one review. Writing the same review twice is a no-op.
    ReviewId
);

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
