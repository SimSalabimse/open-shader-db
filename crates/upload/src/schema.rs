// SPDX-License-Identifier: MIT

//! Hand-maintained JSON Schema for the upload-manifest wire format.
//!
//! `document()` is written by hand and kept aligned with the Rust
//! validators by tests. It is not generated from the serde types.

use crate::manifest::{SCHEMA_VERSION, SCHEMA_VERSION_V1};
use crate::Limits;

/// JSON Schema file checked in at `schemas/upload_manifest.v1.json`.
pub const SCHEMA_JSON: &str = include_str!("../../../schemas/upload_manifest.v1.json");

/// JSON Schema file checked in at `schemas/upload_manifest.v2.json`.
pub const SCHEMA_JSON_V2: &str = include_str!("../../../schemas/upload_manifest.v2.json");

/// Top-level manifest field names for schema version 1, sorted.
pub const MANIFEST_FIELDS: &[&str] = &[
    "build_id",
    "bytecode_hashes",
    "client_version",
    "collected_hour",
    "description_hashes",
    "ownership",
    "schema_version",
];

/// Top-level manifest field names for schema version 2, sorted.
pub const MANIFEST_FIELDS_V2: &[&str] = &[
    "build_id",
    "bytecode_hashes",
    "client_version",
    "collected_hour",
    "description_hashes",
    "ownership",
    "pipeline_hashes",
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
pub const IDENT_PATTERN: &str =
    "^(?!\\.\\.?$)[\\x21-\\x2E\\x30-\\x39\\x3B-\\x5B\\x5D-\\x7E]{1,128}$";

/// Hand-maintained JSON Schema (draft-07). `maxItems` follows [`Limits::DEFAULT`].
///
/// This schema is looser than the Rust and Worker parsers: it does not
/// encode sort order, real calendar dates, byte or nesting caps, or
/// canonical form.
#[must_use]
pub fn document() -> serde_json::Value {
    let max_description = Limits::DEFAULT.max_description_hashes;
    let max_bytecode = Limits::DEFAULT.max_bytecode_hashes;
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "https://github.com/SimSalabimse/open-shader-db/schemas/upload_manifest.v1.json",
        "title": "UploadManifest",
        "description": "Anonymized upload manifest (schema_version 1). No paths, usernames, IPs, hostnames, hardware ids, or full timestamps.",
        "type": "object",
        "additionalProperties": false,
        "required": MANIFEST_FIELDS,
        "properties": {
            "bytecode_hashes": hash_array_schema(max_bytecode),
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
            "description_hashes": hash_array_schema(max_description),
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
                "const": SCHEMA_VERSION_V1
            }
        }
    })
}

/// Hand-maintained JSON Schema for schema version 2.
///
/// Same looseness as [`document`]: no sort order, calendar, or byte caps.
#[must_use]
pub fn document_v2() -> serde_json::Value {
    let max_description = Limits::DEFAULT.max_description_hashes;
    let max_bytecode = Limits::DEFAULT.max_bytecode_hashes;
    let max_pipeline = Limits::DEFAULT.max_pipeline_hashes;
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "https://github.com/SimSalabimse/open-shader-db/schemas/upload_manifest.v2.json",
        "title": "UploadManifest",
        "description": "Anonymized upload manifest (schema_version 2). Adds pipeline_hashes. No paths, usernames, IPs, hostnames, hardware ids, app keys, or full timestamps.",
        "type": "object",
        "additionalProperties": false,
        "required": MANIFEST_FIELDS_V2,
        "properties": {
            "bytecode_hashes": hash_array_schema(max_bytecode),
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
            "description_hashes": hash_array_schema(max_description),
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
            "pipeline_hashes": hash_array_schema(max_pipeline),
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
