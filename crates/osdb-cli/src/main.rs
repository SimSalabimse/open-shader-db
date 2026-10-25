// SPDX-License-Identifier: MIT

//! Thin CLI: inspect a local SODB, or discover Steam games without uploading paths.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use osdb_steam::{discover, discover_default, Discovery};

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
    }
    Ok(())
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
        if let Some(app) = warning.app_id {
            writeln!(err, "warning: {} (app {app})", warning.message)?;
        } else {
            writeln!(err, "warning: {}", warning.message)?;
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
    use osdb_steam::{AppId, BuildId, GameIdentity, GameName, InstalledGame, LocalPath, Warning};

    fn sample_discovery(install: PathBuf) -> Discovery {
        Discovery {
            games: vec![InstalledGame {
                identity: GameIdentity::new(AppId(10), BuildId(99), GameName::new("Sample")),
                install_dir: LocalPath::new(install.clone()),
                sodb_files: vec![LocalPath::new(install.join("a.sodb"))],
            }],
            warnings: vec![Warning {
                kind: osdb_steam::WarningKind::MissingLibraryList,
                app_id: None,
                message: String::from("libraryfolders.vdf was missing or unreadable; using steamapps on the Steam root"),
            }],
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
}
