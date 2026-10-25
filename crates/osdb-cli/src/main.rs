// SPDX-License-Identifier: MIT

//! Thin CLI: inspect a local SODB, discover Steam games, compare coverage, or extract records.

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
        }
    }
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
    }
}
