/// Milliseconds since the Unix epoch, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

const MS_PER_DAY: f64 = 86_400_000.0;

impl Timestamp {
    pub const fn from_millis(ms: i64) -> Self {
        Self(ms)
    }

    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Fractional days from `earlier` to `self`, never negative.
    pub fn days_since(self, earlier: Timestamp) -> f32 {
        ((self.0 - earlier.0).max(0) as f64 / MS_PER_DAY) as f32
    }

    #[must_use]
    pub fn plus_days(self, days: f32) -> Self {
        Self(self.0 + (f64::from(days) * MS_PER_DAY).round() as i64)
    }

    /// Milliseconds from `earlier` to `self`, clamped to `0..=u32::MAX`.
    pub fn millis_since(self, earlier: Timestamp) -> u32 {
        (self.0 - earlier.0).clamp(0, i64::from(u32::MAX)) as u32
    }
}
