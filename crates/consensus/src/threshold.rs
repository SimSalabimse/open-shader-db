// SPDX-License-Identifier: MIT

//! Upload threshold. The library never substitutes a lower bar when a build
//! has few uploads.

use crate::Error;

/// How many counted uploads must carry a hash, and at what share.
///
/// `min_ratio_per_mille` is parts per thousand of the counted uploads that
/// carry the agreement unit (500 is 50%). Constructors reject `min_uploads`
/// of 0 and a ratio above 1000. Fields stay private so a struct literal
/// cannot bypass that check.
///
/// [`crate::build_consensus`] uses the values it is given. It does not lower
/// them for a small build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgreementThreshold {
    min_uploads: u32,
    min_ratio_per_mille: u16,
}

impl AgreementThreshold {
    /// Default bar: 3 counted uploads and 500 per mille (50%).
    pub const DEFAULT_MIN_UPLOADS: u32 = 3;

    /// Default share of the unit's denominator, in parts per thousand.
    pub const DEFAULT_MIN_RATIO_PER_MILLE: u16 = 500;

    /// The documented default. Same numbers as [`Self::new`]`(3, 500)`.
    pub const DEFAULT: Self = Self {
        min_uploads: Self::DEFAULT_MIN_UPLOADS,
        min_ratio_per_mille: Self::DEFAULT_MIN_RATIO_PER_MILLE,
    };

    /// Validated constructor.
    ///
    /// `min_uploads` must be at least 1. `min_ratio_per_mille` must be at
    /// most 1000. A ratio of 0 is valid and means the count alone decides.
    pub const fn new(min_uploads: u32, min_ratio_per_mille: u16) -> Result<Self, Error> {
        if min_uploads == 0 || min_ratio_per_mille > 1000 {
            return Err(Error::InvalidThreshold);
        }
        Ok(Self {
            min_uploads,
            min_ratio_per_mille,
        })
    }

    /// Minimum counted uploads that must list the hash.
    #[must_use]
    pub const fn min_uploads(self) -> u32 {
        self.min_uploads
    }

    /// Minimum share, in parts per thousand, of uploads that carry the unit.
    #[must_use]
    pub const fn min_ratio_per_mille(self) -> u16 {
        self.min_ratio_per_mille
    }
}

impl Default for AgreementThreshold {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Which staged uploads enter a tally.
///
/// Worker uploads are held by default. The challenge endpoint does not exist
/// yet, so this library does not treat an ownership field as evidence.
/// Reviewed-PR uploads are counted. Held uploads are returned and never
/// tallied.
///
/// Turning a flag on does not verify a proof. It only stops holding that
/// class of Worker upload. There is still no account and no check that two
/// uploads came from different people or machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountingPolicy {
    /// Count Worker-staging uploads whose ownership kind is `unchallenged`.
    /// Default `false` (those uploads are held).
    pub count_worker_unchallenged: bool,
    /// Count Worker-staging uploads whose ownership kind is `exe_binding_v1`.
    /// Default `false`. This crate does not check the response bytes.
    pub count_worker_exe_binding: bool,
}

impl CountingPolicy {
    /// Hold every Worker upload. Reviewed-PR uploads still count.
    #[must_use]
    pub const fn hold_worker_uploads() -> Self {
        Self {
            count_worker_unchallenged: false,
            count_worker_exe_binding: false,
        }
    }
}

impl Default for CountingPolicy {
    fn default() -> Self {
        Self::hold_worker_uploads()
    }
}

/// `uploads * 1000 >= min_ratio_per_mille * denominator`, plus the minimum count.
///
/// Products use `u64`. Inputs are `u32` and a ratio of at most 1000, so both
/// products fit. `checked_mul` still fails closed if that ever changes.
pub(crate) fn hash_accepted(uploads: u32, denominator: u32, threshold: AgreementThreshold) -> bool {
    if denominator == 0 || uploads < threshold.min_uploads {
        return false;
    }
    let Some(left) = u64::from(uploads).checked_mul(1000) else {
        return false;
    };
    let Some(right) = u64::from(threshold.min_ratio_per_mille).checked_mul(u64::from(denominator))
    else {
        return false;
    };
    left >= right
}
