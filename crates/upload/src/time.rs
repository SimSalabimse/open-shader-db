// SPDX-License-Identifier: MIT

//! Hour-boundary UTC timestamps. The original instant is never stored.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::Error;

/// A UTC timestamp that can only hold an hour boundary.
///
/// Serializes as `YYYY-MM-DDTHH:00:00Z`. Rounding down happens in the
/// constructor; the original instant is discarded.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcHour {
    text: String,
}

/// Source of the current time. Tests inject a fixed instant.
pub trait Clock {
    /// Current time. Implementations must not print the value.
    fn now(&self) -> SystemTime;
}

/// Operating-system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Test clock that always returns the same instant.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    now: SystemTime,
}

impl FixedClock {
    /// Hold `now` for every call.
    #[must_use]
    pub const fn new(now: SystemTime) -> Self {
        Self { now }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.now
    }
}

impl UtcHour {
    /// Floor `time` to the UTC hour and discard minutes, seconds, and subseconds.
    pub fn from_system_time(time: SystemTime) -> Result<Self, Error> {
        let secs = time
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::TimestampOutOfRange)?
            .as_secs();
        Self::from_unix_secs(secs)
    }

    /// Floor Unix seconds to the UTC hour.
    pub fn from_unix_secs(secs: u64) -> Result<Self, Error> {
        let floored = secs - (secs % 3600);
        let days = floored / 86_400;
        let hour = u8::try_from((floored / 3600) % 24).map_err(|_| Error::TimestampOutOfRange)?;
        let (year, month, day) = days_to_ymd(days)?;
        Self::from_ymd_h(year, month, day, hour)
    }

    /// Construct an hour that is already on a boundary. Minutes are not accepted.
    pub fn from_ymd_h(year: i32, month: u8, day: u8, hour: u8) -> Result<Self, Error> {
        if !(1970..=9999).contains(&year) || hour > 23 {
            return Err(Error::TimestampOutOfRange);
        }
        let year_u = u32::try_from(year).map_err(|_| Error::TimestampOutOfRange)?;
        if !(1..=12).contains(&month) || day < 1 || u32::from(day) > days_in_month(year_u, month) {
            return Err(Error::TimestampOutOfRange);
        }
        Ok(Self {
            text: format!("{year:04}-{month:02}-{day:02}T{hour:02}:00:00Z"),
        })
    }

    /// Parse the hour-boundary form only. Full timestamps are rejected.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let bytes = text.as_bytes();
        if bytes.len() != 20
            || bytes[4] != b'-'
            || bytes[7] != b'-'
            || bytes[10] != b'T'
            || bytes[13] != b':'
            || bytes[14] != b'0'
            || bytes[15] != b'0'
            || bytes[16] != b':'
            || bytes[17] != b'0'
            || bytes[18] != b'0'
            || (bytes[19] != b'Z' && bytes[19] != b'z')
        {
            return Err(Error::InvalidTimestamp);
        }
        let year = parse_digits(&bytes[0..4]).ok_or(Error::InvalidTimestamp)?;
        let month = u8::try_from(parse_digits(&bytes[5..7]).ok_or(Error::InvalidTimestamp)?)
            .map_err(|_| Error::InvalidTimestamp)?;
        let day = u8::try_from(parse_digits(&bytes[8..10]).ok_or(Error::InvalidTimestamp)?)
            .map_err(|_| Error::InvalidTimestamp)?;
        let hour = u8::try_from(parse_digits(&bytes[11..13]).ok_or(Error::InvalidTimestamp)?)
            .map_err(|_| Error::InvalidTimestamp)?;
        let parsed =
            Self::from_ymd_h(year, month, day, hour).map_err(|_| Error::InvalidTimestamp)?;
        if parsed.text != text {
            return Err(Error::InvalidTimestamp);
        }
        Ok(parsed)
    }

    /// Hour-boundary text (`YYYY-MM-DDTHH:00:00Z`).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for UtcHour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Display for UtcHour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Serialize for UtcHour {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

fn parse_digits(bytes: &[u8]) -> Option<i32> {
    let mut value = 0_i32;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(i32::from(byte - b'0'));
    }
    Some(value)
}

fn days_to_ymd(mut days: u64) -> Result<(i32, u8, u8), Error> {
    let mut year = 1970_i32;
    loop {
        let year_u = u32::try_from(year).map_err(|_| Error::TimestampOutOfRange)?;
        let diy = if is_leap_year(year_u) { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year = year.checked_add(1).ok_or(Error::TimestampOutOfRange)?;
        if year > 9999 {
            return Err(Error::TimestampOutOfRange);
        }
    }
    let year_u = u32::try_from(year).map_err(|_| Error::TimestampOutOfRange)?;
    let mut month = 1_u8;
    while month <= 12 {
        let dim = u64::from(days_in_month(year_u, month));
        if days < dim {
            let day = u8::try_from(days + 1).map_err(|_| Error::TimestampOutOfRange)?;
            return Ok((year, month, day));
        }
        days -= dim;
        month = month.checked_add(1).ok_or(Error::TimestampOutOfRange)?;
    }
    Err(Error::TimestampOutOfRange)
}

fn days_in_month(year: u32, month: u8) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}
