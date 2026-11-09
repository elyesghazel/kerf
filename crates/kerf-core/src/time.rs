//! Timeline time model.
//!
//! All positions and durations on the timeline are stored as **microseconds**
//! (`i64`). That is frame-accurate down to 1µs — far finer than any real frame
//! rate — while staying a plain integer, so edits never accumulate float drift.

use serde::{Deserialize, Serialize};

/// A point in time or a duration on the timeline, in microseconds.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Time(pub i64);

impl Time {
    pub const ZERO: Time = Time(0);

    pub const fn from_micros(us: i64) -> Self {
        Time(us)
    }

    pub fn from_secs_f64(secs: f64) -> Self {
        Time((secs * 1_000_000.0).round() as i64)
    }

    /// The timeline position of frame `n` at the given frame rate.
    pub fn from_frame(frame: i64, fps: f64) -> Self {
        Time::from_secs_f64(frame as f64 / fps)
    }

    pub const fn micros(self) -> i64 {
        self.0
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }

    /// Which frame index this instant falls on at the given frame rate.
    pub fn frame_at(self, fps: f64) -> i64 {
        (self.as_secs_f64() * fps).floor() as i64
    }

    /// Snap to the nearest frame boundary at `fps`.
    pub fn round_to_frame(self, fps: f64) -> Self {
        Time::from_frame((self.as_secs_f64() * fps).round() as i64, fps)
    }

    /// `HH:MM:SS:FF` timecode.
    pub fn timecode(self, fps: f64) -> String {
        let neg = self.0 < 0;
        let total = (self.0.abs() as f64 / 1_000_000.0 * fps).round() as i64;
        let fpsi = fps.round().max(1.0) as i64;
        let f = total % fpsi;
        let s = total / fpsi;
        format!(
            "{}{:02}:{:02}:{:02}:{:02}",
            if neg { "-" } else { "" },
            s / 3600,
            (s / 60) % 60,
            s % 60,
            f
        )
    }

    /// Parse seconds (`12.5`), `MM:SS(.ms)`, `HH:MM:SS(.ms)` or timecode with
    /// frames given `fps` (`HH:MM:SS:FF`).
    pub fn parse(s: &str, fps: f64) -> Option<Time> {
        let s = s.trim();
        let s = s.strip_suffix('s').unwrap_or(s);
        let parts: Vec<&str> = s.split(':').collect();
        let secs = match parts.as_slice() {
            [a] => a.parse::<f64>().ok()?,
            [m, s] => m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?,
            [h, m, s] => {
                h.parse::<f64>().ok()? * 3600.0 + m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?
            }
            [h, m, s, f] => {
                h.parse::<f64>().ok()? * 3600.0
                    + m.parse::<f64>().ok()? * 60.0
                    + s.parse::<f64>().ok()?
                    + f.parse::<f64>().ok()? / fps
            }
            _ => return None,
        };
        Some(Time::from_secs_f64(secs))
    }

    pub fn clamp_range(self, min: Time, max: Time) -> Time {
        Time(self.0.clamp(min.0, max.0))
    }
}

impl std::ops::Add for Time {
    type Output = Time;
    fn add(self, rhs: Time) -> Time {
        Time(self.0 + rhs.0)
    }
}

impl std::ops::AddAssign for Time {
    fn add_assign(&mut self, rhs: Time) {
        self.0 += rhs.0;
    }
}

impl std::ops::Neg for Time {
    type Output = Time;
    fn neg(self) -> Time {
        Time(-self.0)
    }
}

impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.3}s", self.as_secs_f64())
    }
}

impl std::ops::Sub for Time {
    type Output = Time;
    fn sub(self, rhs: Time) -> Time {
        Time(self.0 - rhs.0)
    }
}

/// A half-open span `[start, start + duration)` on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Time,
    pub duration: Time,
}

impl TimeRange {
    pub fn new(start: Time, duration: Time) -> Self {
        TimeRange { start, duration }
    }

    pub fn end(self) -> Time {
        self.start + self.duration
    }

    pub fn contains(self, t: Time) -> bool {
        t >= self.start && t < self.end()
    }

    /// Do the two spans share any instant?
    pub fn overlaps(self, other: TimeRange) -> bool {
        self.start < other.end() && other.start < self.end()
    }

    pub fn from_bounds(start: Time, end: Time) -> Self {
        TimeRange::new(start, end - start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        // Frame 30 at 30fps is exactly one second in.
        assert_eq!(Time::from_frame(30, 30.0), Time::from_secs_f64(1.0));
        assert_eq!(Time::from_secs_f64(1.0).frame_at(30.0), 30);
    }

    #[test]
    fn parse_and_timecode() {
        assert_eq!(Time::parse("1:30", 30.0), Some(Time::from_secs_f64(90.0)));
        assert_eq!(Time::parse("2.5s", 30.0), Some(Time::from_secs_f64(2.5)));
        assert_eq!(Time::from_secs_f64(61.5).timecode(30.0), "00:01:01:15");
    }

    #[test]
    fn range_overlap() {
        let a = TimeRange::new(Time(0), Time(100));
        let b = TimeRange::new(Time(50), Time(100));
        let c = TimeRange::new(Time(100), Time(100));
        assert!(a.overlaps(b));
        assert!(!a.overlaps(c)); // half-open: touching at 100 is not overlap
    }
}
