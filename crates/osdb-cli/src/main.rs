// SPDX-License-Identifier: MIT

//! Thin CLI: inspect, discover, coverage, records, local shader scan, rebuild, and upload preview.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use osdb_steam::{discover, discover_default, Discovery};

mod output;

const DEFAULT_MAX_UNMATCHED: usize = 100;

#[derive(Parser)]
#[command(
    name = "osdb-cli",
    version,
    about = "open-shader-db client (Simsalabim)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse an SODB file and print a JSON summary (hashes only, never bytecode).
    Inspect {
        /// Path to a `.sodb` SQLite file.
        file: PathBuf,
    },
    /// List Steam-installed games and how many local SODB files each has.
    Discover {
        /// Steam install root. When omitted, platform default locations are tried.
        #[arg(long)]
        steam_root: Option<PathBuf>,
        /// Print install paths in human output only. JSON never includes paths.
        #[arg(long)]
        show_paths: bool,
        /// Print identities, SODB counts, and warnings as JSON (no paths).
        #[arg(long)]
        json: bool,
    },
    /// Extract pipeline-state records from a local SODB (hashes only, never bytecode).
    Records {
        #[command(subcommand)]
        command: RecordsCommand,
    },
    /// Compare a local SODB to an unsigned coverage index (hashes and counts only).
    Coverage {
        /// Local SODB file. The path is never written into the report.
        #[arg(long)]
        sodb: PathBuf,
        /// Coverage index JSON file. Treated as untrusted and unsigned.
        #[arg(long)]
        index: PathBuf,
        /// Print the coverage report as JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Maximum unmatched hashes to list (the omitted count is always shown).
        #[arg(long, default_value_t = DEFAULT_MAX_UNMATCHED)]
        max_unmatched: usize,
    },
    /// Scan local shaders (SODB or loose DXBC). Report is counts only.
    Shaders {
        #[command(subcommand)]
        command: ShadersCommand,
    },
    /// Build an anonymized upload manifest and show a preview. Nothing is sent.
    Upload {
        #[command(subcommand)]
        command: UploadCommand,
    },
    /// Rebuild a local SODB from records plus local shaders.
    ///
    /// The output contains game bytecode and is for local use only.
    Rebuild {
        /// Record file (hashes and descriptors only).
        #[arg(long)]
        records: PathBuf,
        /// Directory tree of loose DXBC files (repeatable).
        #[arg(long)]
        dir: Vec<PathBuf>,
        /// SODB file the user already has (repeatable).
        #[arg(long)]
        sodb: Vec<PathBuf>,
        /// Destination `.sodb`. Must not be an input or inside a scan root.
        #[arg(long)]
        out: PathBuf,
        /// Overwrite `--out` if it already exists as a regular file.
        #[arg(long)]
        force: bool,
        /// Print the rebuild report as JSON on stdout (counts only).
        #[arg(long)]
        json: bool,
        /// Cap on the rebuilt file in bytes (default 8 GiB, hard maximum 8 GiB).
        #[arg(long, default_value_t = osdb_rebuild::DEFAULT_MAX_OUTPUT_BYTES)]
        max_output_bytes: u64,
    },
}

impl fmt::Debug for Cli {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.command {
            Command::Inspect { file: _ } => f
                .debug_struct("Inspect")
                .field("file", &"<redacted>")
                .finish(),
            Command::Discover {
                steam_root: _,
                show_paths,
                json,
            } => f
                .debug_struct("Discover")
                .field("steam_root", &"<redacted>")
                .field("show_paths", show_paths)
                .field("json", json)
                .finish(),
            Command::Coverage {
                sodb: _,
                index: _,
                json,
                max_unmatched,
            } => f
                .debug_struct("Coverage")
                .field("sodb", &"<redacted>")
                .field("index", &"<redacted>")
                .field("json", json)
                .field("max_unmatched", max_unmatched)
                .finish(),
            Command::Records { command } => match command {
                RecordsCommand::Extract {
                    sodb: _,
                    out: _,
                    game_id,
                    version,
                    exe_hash,
                    force,
                    hash_app_keys,
                } => f
                    .debug_struct("RecordsExtract")
                    .field("sodb", &"<redacted>")
                    .field("out", &"<redacted>")
                    .field("game_id", game_id)
                    .field("version", version)
                    .field("exe_hash", exe_hash)
                    .field("force", force)
                    .field("hash_app_keys", hash_app_keys)
                    .finish(),
            },
            Command::Shaders { command } => match command {
                ShadersCommand::Scan {
                    dir: _,
                    sodb: _,
                    json,
                    out: _,
                    force,
                } => f
                    .debug_struct("ShadersScan")
                    .field("dir", &"<redacted>")
                    .field("sodb", &"<redacted>")
                    .field("json", json)
                    .field("out", &"<redacted>")
                    .field("force", force)
                    .finish(),
            },
            Command::Rebuild {
                records: _,
                dir: _,
                sodb: _,
                out: _,
                force,
                json,
                max_output_bytes,
            } => f
                .debug_struct("Rebuild")
                .field("records", &"<redacted>")
                .field("dir", &"<redacted>")
                .field("sodb", &"<redacted>")
                .field("out", &"<redacted>")
                .field("force", force)
                .field("json", json)
                .field("max_output_bytes", max_output_bytes)
                .finish(),
            Command::Upload { command } => match command {
                UploadCommand::Prepare {
                    records: _,
                    build_id,
                    version,
                    exe_hash,
                    out: _,
                    force,
                    yes_i_reviewed,
                } => f
                    .debug_struct("UploadPrepare")
                    .field("records", &"<redacted>")
                    .field("build_id", build_id)
                    .field("version", version)
                    .field("exe_hash", exe_hash)
                    .field("out", &"<redacted>")
                    .field("force", force)
                    .field("yes_i_reviewed", yes_i_reviewed)
                    .finish(),
            },
        }
    }
}

#[derive(Subcommand)]
enum ShadersCommand {
    /// Hash local shaders. Report is counts only; never prints paths or bytecode.
    Scan {
        /// Directory tree of loose files that start with DXBC.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// SODB file the user already has.
        #[arg(long)]
        sodb: Option<PathBuf>,
        /// Print the scan report as JSON (counts only).
        #[arg(long)]
        json: bool,
        /// Write the report to a file (atomic; refuse overwrite without `--force`).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Overwrite `--out` if it already exists as a regular file.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum UploadCommand {
    /// Build a local preview of the upload payload. Nothing is sent.
    Prepare {
        /// Record file from `records extract`.
        #[arg(long)]
        records: PathBuf,
        /// Game id written into `build_id.game_id`.
        #[arg(long)]
        build_id: String,
        /// Optional version override (otherwise taken from the record file).
        #[arg(long)]
        version: Option<String>,
        /// Optional exe SHA-256 override (64 lowercase hex characters).
        #[arg(long)]
        exe_hash: Option<String>,
        /// Write the canonical manifest bytes. Requires `--yes-i-reviewed`.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Overwrite `--out` if it already exists as a regular file.
        #[arg(long)]
        force: bool,
        /// Required with `--out`. Confirms the preview was reviewed.
        #[arg(long)]
        yes_i_reviewed: bool,
    },
}

#[derive(Subcommand)]
enum RecordsCommand {
    /// Write a local record file from an SODB the user already has.
    Extract {
        /// Local SODB file. The path is never written into the output.
        #[arg(long)]
        sodb: PathBuf,
        /// Destination JSON file. When omitted, JSON is written to stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Optional user-supplied game id (same rules as the coverage index).
        #[arg(long)]
        game_id: Option<String>,
        /// Optional user-supplied version string.
        #[arg(long)]
        version: Option<String>,
        /// Optional user-supplied exe SHA-256 (64 lowercase hex characters).
        #[arg(long)]
        exe_hash: Option<String>,
        /// Overwrite `--out` if it already exists as a regular file.
        #[arg(long)]
        force: bool,
        /// Replace each `app_key` with the SHA-256 of the game-provided key.
        ///
        /// The hash is unsalted, so low-entropy keys can be guessed. The
        /// tool does not alter keys unless this flag is set. Hashing hides
        /// path-shaped keys at the cost of making the original key
        /// unrecoverable.
        #[arg(long)]
        hash_app_keys: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect { file } => {
            let summary = sodb::open(&file)?;
            let mut out = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut out, &summary)?;
            writeln!(out)?;
        }
        Command::Discover {
            steam_root,
            show_paths,
            json,
        } => {
            let found = match steam_root.as_deref() {
                Some(root) => discover(root),
                None => discover_default(),
            };
            if json {
                write_json(&found)?;
            } else {
                write_human(&found, show_paths)?;
            }
        }
        Command::Records {
            command:
                RecordsCommand::Extract {
                    sodb,
                    out,
                    game_id,
                    version,
                    exe_hash,
                    force,
                    hash_app_keys,
                },
        } => {
            let build = parse_build(game_id.as_deref(), version.as_deref(), exe_hash.as_deref())?;
            let report = osdb_records::extract_with_report(
                &sodb,
                build,
                sodb::Limits::DEFAULT,
                hash_app_keys,
            )?;
            match out {
                Some(path) => output::write_atomic(&path, &sodb, force, |dest| {
                    osdb_records::encode_record_file_to_writer(&report.file, dest)
                        .map_err(Into::into)
                })?,
                None => {
                    let mut stdout = std::io::stdout().lock();
                    osdb_records::encode_record_file_to_writer(&report.file, &mut stdout)?;
                }
            }
            if let Some(msg) =
                osdb_records::skipped_root_signatures_message(report.skipped_root_signatures)
            {
                eprintln!("{msg}");
            }
        }
        Command::Coverage {
            sodb,
            index,
            json,
            max_unmatched,
        } => {
            let summary = sodb::open(&sodb)?;
            let coverage = osdb_coverage::load(&index)?;
            let report = osdb_coverage::compare(&summary, &coverage, max_unmatched);
            let mut out = std::io::stdout().lock();
            if json {
                serde_json::to_writer_pretty(&mut out, &report)?;
                writeln!(out)?;
            } else {
                write!(out, "{}", report.format_human())?;
            }
        }
        Command::Upload {
            command:
                UploadCommand::Prepare {
                    records,
                    build_id,
                    version,
                    exe_hash,
                    out,
                    force,
                    yes_i_reviewed,
                },
        } => {
            let loaded = osdb_records::load(&records)?;
            let mut builder = osdb_upload::ManifestBuilder::new(&loaded)
                .game_id(osdb_upload::GameId::parse(&build_id)?);
            if let Some(text) = version {
                builder = builder.version(osdb_upload::GameVersion::parse(&text)?);
            }
            if let Some(text) = exe_hash {
                builder = builder.exe_hash(osdb_upload::ExeHash::parse(&text)?);
            }
            let manifest = builder.build()?;
            let preview = osdb_upload::UploadPreview::from_manifest(&manifest)?;
            let mut stdout = std::io::stdout().lock();
            write!(stdout, "{}", preview.as_text())?;
            if let Some(path) = out {
                if !yes_i_reviewed {
                    return Err(
                        "pass --yes-i-reviewed after reviewing the preview; nothing was written"
                            .into(),
                    );
                }
                let bytes = preview.canonical_bytes().to_vec();
                output::write_atomic(&path, &records, force, |dest| {
                    dest.write_all(&bytes).map_err(Into::into)
                })?;
            }
        }
        Command::Shaders {
            command:
                ShadersCommand::Scan {
                    dir,
                    sodb,
                    json,
                    out,
                    force,
                },
        } => {
            let input = match (dir, sodb) {
                (Some(dir), None) => osdb_shaders::ScanInput::Directory(dir),
                (None, Some(sodb)) => osdb_shaders::ScanInput::Sodb(sodb),
                _ => {
                    return Err("shaders scan requires exactly one of --dir or --sodb".into());
                }
            };
            let input_path = match &input {
                osdb_shaders::ScanInput::Directory(p) | osdb_shaders::ScanInput::Sodb(p) => {
                    p.clone()
                }
            };
            let (_index, report) = osdb_shaders::scan(&[input], osdb_shaders::Limits::DEFAULT)?;
            let body = if json {
                let mut bytes = serde_json::to_vec_pretty(&report)?;
                bytes.push(b'\n');
                bytes
            } else {
                report.format_human().into_bytes()
            };
            match out {
                Some(path) => output::write_atomic(&path, &input_path, force, |dest| {
                    dest.write_all(&body).map_err(Into::into)
                })?,
                None => {
                    let mut stdout = std::io::stdout().lock();
                    stdout.write_all(&body)?;
                }
            }
        }
        Command::Rebuild {
            records,
            dir,
            sodb,
            out,
            force,
            json,
            max_output_bytes,
        } => {
            if dir.is_empty() && sodb.is_empty() {
                return Err("rebuild requires at least one --dir or --sodb".into());
            }
            let loaded = osdb_records::load(&records).map_err(|err| format!("records: {err}"))?;
            let mut inputs = Vec::new();
            for path in dir {
                inputs.push(osdb_shaders::ScanInput::Directory(path));
            }
            for path in sodb {
                inputs.push(osdb_shaders::ScanInput::Sodb(path));
            }
            let (index, _) = osdb_shaders::scan(&inputs, osdb_shaders::Limits::DEFAULT)
                .map_err(|err| format!("scan: {err}"))?;
            let opts = osdb_rebuild::RebuildOptions {
                force,
                max_output_bytes,
                record_input: Some(records),
            };
            let report = osdb_rebuild::rebuild_sodb(&loaded, &index, &out, opts)
                .map_err(|err| format!("rebuild: {err}"))?;
            eprintln!("{}", osdb_rebuild::LOCAL_USE_NOTICE);
            if json {
                let mut stdout = std::io::stdout().lock();
                serde_json::to_writer_pretty(&mut stdout, &report)?;
                writeln!(stdout)?;
            } else {
                eprint!("{}", report.format_human());
            }
        }
    }
    Ok(())
}

fn parse_build(
    game_id: Option<&str>,
    version: Option<&str>,
    exe_hash: Option<&str>,
) -> Result<osdb_records::BuildIdentity, osdb_records::Error> {
    let game_id = match game_id {
        Some(text) => Some(osdb_records::GameId::parse(text)?),
        None => None,
    };
    let version = match version {
        Some(text) => Some(osdb_records::GameVersion::parse(text)?),
        None => None,
    };
    let exe_hash = match exe_hash {
        Some(text) => Some(osdb_records::ExeHash::parse_hex(text)?),
        None => None,
    };
    Ok(osdb_records::BuildIdentity::new(game_id, version, exe_hash))
}

fn write_json(found: &Discovery) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &found.upload_safe())?;
    writeln!(out)?;
    Ok(())
}

fn write_human(found: &Discovery, show_paths: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut err = std::io::stderr().lock();
    for warning in &found.warnings {
        if let Some(app) = warning.app_id() {
            writeln!(err, "warning: {} (app {app})", warning.message())?;
        } else {
            writeln!(err, "warning: {}", warning.message())?;
        }
    }

    let mut out = std::io::stdout().lock();
    if found.games.is_empty() {
        writeln!(out, "No games found.")?;
        return Ok(());
    }
    for game in &found.games {
        writeln!(
            out,
            "{}  app={}  build={}  sodb_files={}",
            game.identity.name().as_str(),
            game.identity.app_id(),
            game.identity.build_id(),
            game.sodb_files.len()
        )?;
        if show_paths {
            writeln!(out, "  install: {}", game.install_dir.as_path().display())?;
            for sodb in &game.sodb_files {
                writeln!(out, "  sodb: {}", sodb.as_path().display())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use osdb_steam::{AppId, BuildId, GameIdentity, GameName, InstalledGame, LocalPath};

    fn sample_discovery(install: PathBuf) -> Discovery {
        Discovery {
            games: vec![InstalledGame {
                identity: GameIdentity::new(AppId(10), BuildId(99), GameName::new("Sample")),
                install_dir: LocalPath::new(install.clone()),
                sodb_files: vec![LocalPath::new(install.join("a.sodb"))],
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn json_output_never_includes_paths() {
        let marker = PathBuf::from("/tmp/user-alice-fixture/SteamLibrary/SampleGame");
        let found = sample_discovery(marker.clone());
        let json = serde_json::to_string(&found.upload_safe()).unwrap();
        assert!(!json.contains("user-alice-fixture"));
        assert!(!json.contains("SampleGame"));
        assert!(!json.contains("/tmp/"));
        assert!(json.contains("\"sodb_file_count\":1"));
        assert!(json.contains("Sample"));
    }

    #[test]
    fn debug_redacts_paths() {
        let inspect = Cli {
            command: Command::Inspect {
                file: PathBuf::from("/home/os-user/game.sodb"),
            },
        };
        let text = format!("{inspect:?}");
        assert_eq!(text, r#"Inspect { file: "<redacted>" }"#);
        assert!(!text.contains("os-user"));

        let discover = Cli {
            command: Command::Discover {
                steam_root: Some(PathBuf::from("/home/os-user/.steam")),
                show_paths: false,
                json: true,
            },
        };
        let text = format!("{discover:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("os-user"));

        let coverage = Cli {
            command: Command::Coverage {
                sodb: PathBuf::from("/home/os-user/game.sodb"),
                index: PathBuf::from("/tmp/os-user-index.json"),
                json: true,
                max_unmatched: 4,
            },
        };
        let text = format!("{coverage:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("os-user"));
        assert!(text.contains("json: true"));

        let records = Cli {
            command: Command::Records {
                command: RecordsCommand::Extract {
                    sodb: PathBuf::from("/home/os-user/game.sodb"),
                    out: Some(PathBuf::from("/tmp/os-user-records.json")),
                    game_id: Some(String::from("synthetic")),
                    version: Some(String::from("1")),
                    exe_hash: None,
                    force: false,
                    hash_app_keys: false,
                },
            },
        };
        let text = format!("{records:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("os-user"));
        assert!(text.contains("synthetic"));

        let shaders = Cli {
            command: Command::Shaders {
                command: ShadersCommand::Scan {
                    dir: Some(PathBuf::from("/home/os-user/shaders")),
                    sodb: None,
                    json: true,
                    out: Some(PathBuf::from("/tmp/os-user-scan.json")),
                    force: false,
                },
            },
        };
        let text = format!("{shaders:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("os-user"));

        let rebuild = Cli {
            command: Command::Rebuild {
                records: PathBuf::from("/home/os-user/records.json"),
                dir: vec![PathBuf::from("/home/os-user/game")],
                sodb: Vec::new(),
                out: PathBuf::from("/tmp/os-user-out.sodb"),
                force: true,
                json: false,
                max_output_bytes: osdb_rebuild::DEFAULT_MAX_OUTPUT_BYTES,
            },
        };
        let text = format!("{rebuild:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("os-user"));
        assert!(text.contains("force: true"));

        let upload = Cli {
            command: Command::Upload {
                command: UploadCommand::Prepare {
                    records: PathBuf::from("/home/os-user/records.json"),
                    build_id: String::from("synthetic"),
                    version: Some(String::from("1")),
                    exe_hash: None,
                    out: Some(PathBuf::from("/tmp/os-user-manifest.json")),
                    force: false,
                    yes_i_reviewed: true,
                },
            },
        };
        let text = format!("{upload:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("os-user"));
        assert!(text.contains("synthetic"));
        assert!(text.contains("yes_i_reviewed: true"));
    }
}
