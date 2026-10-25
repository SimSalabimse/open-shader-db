// SPDX-License-Identifier: MIT

//! JSON Schema document derived from the Rust types in this crate.

use crate::manifest::SCHEMA_VERSION;
use crate::Limits;

/// JSON Schema file checked in at `schemas/upload_manifest.v1.json`.
pub const SCHEMA_JSON: &str = include_str!("../../../schemas/upload_manifest.v1.json");

/// Top-level manifest field names, sorted, matching the wire allowlist.
pub const MANIFEST_FIELDS: &[&str] = &[
    "build_id",
    "bytecode_hashes",
    "client_version",
    "collected_hour",
    "description_hashes",
    "ownership",
    "schema_version",
];

/// `build_id` object field names, sorted.
pub const BUILD_ID_FIELDS: &[&str] = &["exe_hash", "game_id", "version"];

/// SHA-256 hex pattern used by constructors and the schema.
pub const SHA256_HEX_PATTERN: &str = "^[0-9a-f]{64}$";

/// Hour-boundary UTC pattern. Minutes and seconds must be `00`.
pub const UTC_HOUR_PATTERN: &str = "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:00:00Z$";

/// Client version pattern (`major.minor.patch` of digits).
pub const CLIENT_VERSION_PATTERN: &str = "^[0-9]+\\.[0-9]+\\.[0-9]+$";

/// Game id / version identifier pattern (ASCII graphic minus `/`, `\\`, `:`).
pub const IDENT_PATTERN: &str = "^[\\x21-\\x2E\\x30-\\x39\\x3B-\\x5B\\x5D-\\x7E]{1,128}$";

/// JSON Schema (draft-07) generated from the Rust types and limits.
#[must_use]
pub fn document() -> serde_json::Value {
    let max_hashes = Limits::DEFAULT.max_hashes_per_list;
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "https://github.com/SimSalabimse/open-shader-db/schemas/upload_manifest.v1.json",
        "title": "UploadManifest",
        "description": "Anonymized upload manifest (schema_version 1). No paths, usernames, IPs, hostnames, hardware ids, or full timestamps.",
        "type": "object",
        "additionalProperties": false,
        "required": MANIFEST_FIELDS,
        "properties": {
            "bytecode_hashes": hash_array_schema(max_hashes),
            "build_id": {
                "type": "object",
                "additionalProperties": false,
                "required": BUILD_ID_FIELDS,
                "properties": {
                    "exe_hash": sha256_schema(),
                    "game_id": ident_schema(),
                    "version": ident_schema()
                }
            },
            "client_version": {
                "type": "string",
                "pattern": CLIENT_VERSION_PATTERN,
                "maxLength": 32
            },
            "collected_hour": {
                "type": "string",
                "pattern": UTC_HOUR_PATTERN,
                "description": "UTC hour boundary. Minutes and seconds are always 00."
            },
            "description_hashes": hash_array_schema(max_hashes),
            "ownership": {
                "oneOf": [
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "reason"],
                        "properties": {
                            "kind": { "const": "unchallenged" },
                            "reason": { "const": "no_server_challenge" }
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["challenge", "kind", "response"],
                        "properties": {
                            "challenge": sha256_schema(),
                            "kind": { "const": "exe_binding_v1" },
                            "response": sha256_schema()
                        }
                    }
                ]
            },
            "schema_version": {
                "type": "integer",
                "const": SCHEMA_VERSION
            }
        }
    })
}

fn hash_array_schema(max_hashes: u64) -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "uniqueItems": true,
        "maxItems": max_hashes,
        "items": sha256_schema()
    })
}

fn sha256_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "pattern": SHA256_HEX_PATTERN,
        "minLength": 64,
        "maxLength": 64
    })
}

fn ident_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "pattern": IDENT_PATTERN,
        "minLength": 1,
        "maxLength": 128
    })
}
