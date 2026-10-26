// SPDX-License-Identifier: MIT

//! Shared golden vectors: every file in `schemas/vectors/` is parsed here.

#![deny(clippy::unwrap_used, clippy::expect_used)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use osdb_upload::{
    BuildId, ClientVersion, Error, ExeHash, GameId, GameVersion, OwnershipProof, Sha256Hex,
    UploadManifest, UtcHour,
};

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/vectors")
}

fn relative_unix(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    visit(dir, &mut out);
    out.retain(|path| {
        let rel = relative_unix(dir, path);
        path.file_name().and_then(|n| n.to_str()) != Some("index.json")
            && !rel.starts_with("stored/")
    });
    out.sort_by_key(|left| relative_unix(dir, left));
    out
}

fn stored_files(dir: &Path) -> Vec<PathBuf> {
    let stored = dir.join("stored");
    if !stored.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    visit(&stored, &mut out);
    out.sort_by_key(|left| relative_unix(dir, left));
    out
}

#[derive(Debug, serde::Deserialize)]
struct VectorIndex {
    count: usize,
    vectors: Vec<VectorIndexEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct VectorIndexEntry {
    path: String,
    verdict: String,
    error_code: Option<String>,
    stored: Option<String>,
}

fn load_index(dir: &Path) -> VectorIndex {
    let text = fs::read_to_string(dir.join("index.json"))
        .unwrap()
        .replace("\r\n", "\n");
    serde_json::from_str(&text).unwrap()
}

fn visit(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            visit(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

fn decode_hex(text: &str) -> Vec<u8> {
    assert!(
        text.len().is_multiple_of(2),
        "hex length must be even in {}",
        text.len()
    );
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i]);
        let lo = hex_nibble(bytes[i + 1]);
        out.push((hi << 4) | lo);
        i += 2;
    }
    out
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => panic!("non-lowercase hex in vector body_hex"),
    }
}

fn payload(value: &serde_json::Value, path: &Path) -> Vec<u8> {
    let has_body = value.get("body").is_some();
    let has_text = value.get("body_text").is_some();
    let has_hex = value.get("body_hex").is_some();
    assert_eq!(
        usize::from(has_body) + usize::from(has_text) + usize::from(has_hex),
        1,
        "{} must have exactly one of body, body_text, body_hex",
        path.display()
    );
    if has_hex {
        return decode_hex(value["body_hex"].as_str().unwrap());
    }
    if has_text {
        return value["body_text"].as_str().unwrap().as_bytes().to_vec();
    }
    serde_json::to_vec(&value["body"]).unwrap()
}

#[test]
fn every_golden_vector_matches_rust_parser() {
    let dir = vectors_dir();
    let files = json_files(&dir);
    let index = load_index(&dir);
    assert_eq!(
        index.count,
        index.vectors.len(),
        "index count must match the vector list"
    );
    let relative: Vec<String> = files
        .iter()
        .map(|path| {
            path.strip_prefix(&dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    let listed: Vec<String> = index
        .vectors
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(
        relative, listed,
        "on-disk vectors must match schemas/vectors/index.json exactly"
    );
    let accept_stored: Vec<String> = index
        .vectors
        .iter()
        .filter(|entry| entry.verdict == "accept")
        .map(|entry| {
            entry
                .stored
                .clone()
                .unwrap_or_else(|| panic!("{} accept entry must list stored bytes", entry.path))
        })
        .collect();
    let on_disk_stored: Vec<String> = stored_files(&dir)
        .iter()
        .map(|path| relative_unix(&dir, path))
        .collect();
    assert_eq!(
        accept_stored.len(),
        index
            .vectors
            .iter()
            .filter(|entry| entry.verdict == "accept")
            .count(),
        "every accept vector must have a stored-bytes path"
    );
    assert_eq!(
        on_disk_stored, accept_stored,
        "on-disk stored-bytes files must match accept index entries exactly"
    );
    for (path, listed) in files.iter().zip(index.vectors.iter()) {
        let text = fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let verdict = value["verdict"].as_str().unwrap();
        let error_code = value["error_code"].as_str();
        assert_eq!(
            verdict,
            listed.verdict,
            "{} verdict must match index",
            path.display()
        );
        assert_eq!(
            error_code,
            listed.error_code.as_deref(),
            "{} error_code must match index",
            path.display()
        );
        let bytes = payload(&value, path);
        let parsed = UploadManifest::parse(&bytes);
        match verdict {
            "accept" => {
                assert!(
                    error_code.is_none(),
                    "{} accept vector must use error_code null",
                    path.display()
                );
                let manifest = parsed.unwrap_or_else(|err| {
                    panic!(
                        "{} should accept, got {} ({err})",
                        path.display(),
                        err.code()
                    )
                });
                let canonical = manifest.to_canonical_bytes().unwrap();
                if value.get("body").is_some() {
                    assert_eq!(
                        canonical,
                        bytes,
                        "{} accept body must be crate-canonical",
                        path.display()
                    );
                }
                let stored_rel = listed.stored.as_deref().unwrap_or_else(|| {
                    panic!(
                        "{} accept index entry must list stored bytes",
                        path.display()
                    )
                });
                let stored_path = dir.join(stored_rel);
                let expected = fs::read(&stored_path).unwrap_or_else(|err| {
                    panic!(
                        "{} missing stored bytes {}: {err}",
                        path.display(),
                        stored_rel
                    )
                });
                assert_eq!(
                    canonical,
                    expected,
                    "{} stored bytes must match the Rust canonicalizer",
                    path.display()
                );
            }
            "reject" => {
                let expected = error_code
                    .unwrap_or_else(|| panic!("{} reject vector needs error_code", path.display()));
                let err = parsed.expect_err(&format!("{} should reject", path.display()));
                assert_eq!(
                    err.code(),
                    expected,
                    "{} expected {expected}, got {} ({err})",
                    path.display(),
                    err.code()
                );
            }
            other => panic!("{} unknown verdict {other}", path.display()),
        }
    }
}

#[test]
fn valid_vectors_are_generated_from_the_crate() {
    let dir = vectors_dir().join("valid");
    for path in json_files(&dir) {
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap().replace("\r\n", "\n"))
                .unwrap();
        assert_eq!(value["verdict"], "accept");
        let body = value.get("body").unwrap_or_else(|| {
            panic!(
                "{} valid vector must use body from the crate",
                path.display()
            )
        });
        let parsed = UploadManifest::parse(&serde_json::to_vec(body).unwrap()).unwrap();
        let rebuilt = rebuild_from_typed(&parsed);
        assert_eq!(
            rebuilt.to_canonical_bytes().unwrap(),
            parsed.to_canonical_bytes().unwrap(),
            "{} must round-trip through typed constructors",
            path.display()
        );
    }
}

fn rebuild_from_typed(parsed: &UploadManifest) -> UploadManifest {
    let build = BuildId::new(
        GameId::parse(parsed.build_id.game_id.as_str()).unwrap(),
        GameVersion::parse(parsed.build_id.version.as_str()).unwrap(),
        ExeHash::parse(&parsed.build_id.exe_hash.to_hex()).unwrap(),
    );
    UploadManifest {
        schema_version: parsed.schema_version,
        build_id: build,
        client_version: ClientVersion::parse(parsed.client_version.as_str()).unwrap(),
        collected_hour: UtcHour::parse(parsed.collected_hour.as_str()).unwrap(),
        description_hashes: parsed.description_hashes.clone(),
        bytecode_hashes: parsed.bytecode_hashes.clone(),
        ownership: match &parsed.ownership {
            OwnershipProof::Unchallenged { .. } => OwnershipProof::unchallenged(),
            OwnershipProof::ExeBindingV1 {
                challenge,
                response,
            } => OwnershipProof::exe_binding_v1(
                Sha256Hex::parse(&challenge.to_hex()).unwrap(),
                Sha256Hex::parse(&response.to_hex()).unwrap(),
            ),
        },
    }
}

#[test]
#[ignore = "run with OSDB_WRITE_STORED=1 to refresh stored-byte oracles"]
fn write_stored_byte_oracles() {
    assert_eq!(
        std::env::var("OSDB_WRITE_STORED").ok().as_deref(),
        Some("1"),
        "refusing to write stored bytes without OSDB_WRITE_STORED=1"
    );
    let dir = vectors_dir();
    let files = json_files(&dir);
    for path in files {
        let text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        if value["verdict"].as_str() != Some("accept") {
            continue;
        }
        let bytes = payload(&value, &path);
        let manifest = UploadManifest::parse(&bytes).unwrap_or_else(|err| {
            panic!(
                "{} should accept while writing stored bytes: {err}",
                path.display()
            )
        });
        let canonical = manifest.to_canonical_bytes().unwrap();
        let rel = relative_unix(&dir, &path);
        let stored = dir.join("stored").join(&rel);
        if let Some(parent) = stored.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&stored, canonical).unwrap();
    }
}

#[test]
fn error_codes_are_stable_machine_strings() {
    assert_eq!(
        Error::UnsortedOrDuplicateHashes.code(),
        "UNSORTED_OR_DUPLICATE_HASHES"
    );
    assert_eq!(Error::InvalidTimestamp.code(), "INVALID_TIMESTAMP");
    assert_eq!(Error::UnknownOwnershipKind.code(), "UNKNOWN_OWNERSHIP_KIND");
}
