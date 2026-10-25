// SPDX-License-Identifier: MIT

//! Versioned proof-of-ownership field. See docs/UPLOAD_MANIFEST.md.

use std::fmt;

use serde::Serialize;

use crate::hash::Sha256Hex;

/// Why a manifest has no challenge-response yet.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnchallengedReason {
    /// No ingest server has issued a nonce (issues #12 and #16).
    NoServerChallenge,
}

impl UnchallengedReason {
    /// Documented reason code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoServerChallenge => "no_server_challenge",
        }
    }
}

impl fmt::Debug for UnchallengedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for UnchallengedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Proof-of-ownership field. Versioned so the Worker can validate one schema.
///
/// The client in this crate emits only [`OwnershipProof::Unchallenged`].
/// [`OwnershipProof::ExeBindingV1`] is part of the schema so a later
/// server challenge can be checked without a second type.
#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind")]
pub enum OwnershipProof {
    /// Missing-challenge state. Proves nothing. Preview shows this plainly.
    #[serde(rename = "unchallenged")]
    Unchallenged { reason: UnchallengedReason },
    /// Challenge-response bound to game executable bytes.
    ///
    /// `response` is SHA-256(`challenge` bytes || exe file bytes), streamed
    /// on the client. This variant is accepted on parse and is **not**
    /// produced by [`crate::ManifestBuilder`] until a server nonce exists.
    #[serde(rename = "exe_binding_v1")]
    ExeBindingV1 {
        /// Server-issued 32-byte challenge, 64 lowercase hex.
        challenge: Sha256Hex,
        /// SHA-256 of challenge bytes concatenated with exe file bytes.
        response: Sha256Hex,
    },
}

impl OwnershipProof {
    /// Well-formed field for a client that has no server challenge.
    #[must_use]
    pub const fn unchallenged() -> Self {
        Self::Unchallenged {
            reason: UnchallengedReason::NoServerChallenge,
        }
    }

    /// Schema-ready future variant. Not emitted by the prepare command.
    #[must_use]
    pub const fn exe_binding_v1(challenge: Sha256Hex, response: Sha256Hex) -> Self {
        Self::ExeBindingV1 {
            challenge,
            response,
        }
    }

    /// Wire `kind` string.
    #[must_use]
    pub const fn kind_str(&self) -> &'static str {
        match self {
            Self::Unchallenged { .. } => "unchallenged",
            Self::ExeBindingV1 { .. } => "exe_binding_v1",
        }
    }
}

impl fmt::Debug for OwnershipProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unchallenged { reason } => f
                .debug_struct("Unchallenged")
                .field("reason", reason)
                .finish(),
            Self::ExeBindingV1 {
                challenge,
                response,
            } => f
                .debug_struct("ExeBindingV1")
                .field("challenge", challenge)
                .field("response", response)
                .finish(),
        }
    }
}

impl Default for OwnershipProof {
    fn default() -> Self {
        Self::unchallenged()
    }
}
