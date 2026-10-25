// SPDX-License-Identifier: MIT

//! Thin CLI: inspect a local SODB or compare it to a coverage index.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
    /// Compare a local SODB to an unsigned coverage index (hashes and counts only).
    Coverage {
        /// Local SODB file. The path is never written into the report.
        #[arg(long)]
        sodb: PathBuf,
        /// Coverage index JSON file. Treated as untrusted and unsigned.
        #[arg(long)]
        index: PathBuf,
        /// Print [`osdb_coverage::CoverageReport`] as JSON instead of text.
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
        }
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

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
    }
}
