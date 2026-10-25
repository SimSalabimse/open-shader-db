// SPDX-License-Identifier: MIT

//! Client version newtype (`major.minor.patch` of digits).

use std::fmt;

use serde::Serialize;

use crate::Error;

/// open-shader-db client version. Wire form is `major.minor.patch`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ClientVersion(String);

impl ClientVersion {
    /// Version of this crate (`CARGO_PKG_VERSION`).
    pub fn current() -> Result<Self, Error> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }

    /// Validate `major.minor.patch` of ASCII digits (max 32 characters).
    pub fn parse(text: &str) -> Result<Self, Error> {
        if text.is_empty() || text.len() > 32 {
            return Err(Error::InvalidClientVersion);
        }
        let mut parts = text.split('.');
        let major = parts.next().ok_or(Error::InvalidClientVersion)?;
        let minor = parts.next().ok_or(Error::InvalidClientVersion)?;
        let patch = parts.next().ok_or(Error::InvalidClientVersion)?;
        if parts.next().is_some() {
            return Err(Error::InvalidClientVersion);
        }
        for part in [major, minor, patch] {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::InvalidClientVersion);
            }
        }
        Ok(Self(text.to_string()))
    }

    /// Version text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ClientVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for ClientVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
