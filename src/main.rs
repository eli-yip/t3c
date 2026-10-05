mod complete;
mod database;
mod output;

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Search and complete Things 3 to-dos")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Complete one or more full to-do IDs, in order, and verify each result
    #[command(
        after_help = "Duplicate IDs are processed once. Each ID is attempted even if another fails.\nWith multiple input IDs, --json returns results and succeeded/failed/unconfirmed counts.\nExit code: 0 if all succeed, 1 for any failure or unconfirmed result.\nSingle-ID errors remain text on stderr, including with --json."
    )]
    Complete {
        #[arg(required = true, num_args = 1.., value_name = "ID", value_parser = nonblank)]
        ids: Vec<String>,
        /// Output the completion or batch results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Find to-dos containing a literal phrase (ASCII case-insensitive)
    Search {
        #[arg(value_parser = nonblank)]
        query: String,
        /// Output complete matching fields as JSON
        #[arg(long)]
        json: bool,
        /// Maximum to-dos to return (default: all)
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        limit: Option<i64>,
        /// Skip this many matching to-dos
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..))]
        offset: i64,
        /// Include to-dos in the trash, including trashed projects
        #[arg(long)]
        include_trashed: bool,
        /// Include completed to-dos and checked checklist items
        #[arg(long)]
        include_completed: bool,
        /// Include canceled to-dos
        #[arg(long)]
        include_canceled: bool,
    },
}

fn nonblank(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("query must contain non-whitespace characters".into())
    } else {
        Ok(value.to_owned())
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let mut exit = ExitCode::SUCCESS;
    let mut out = io::BufWriter::new(io::stdout().lock());
    match cli.command {
        Command::Search {
            query,
            json,
            limit,
            offset,
            include_trashed,
            include_completed,
            include_canceled,
        } => {
            let path = database::locate()?;
            let result = database::search(
                &path,
                &query,
                limit,
                offset,
                include_trashed,
                include_completed,
                include_canceled,
            )?;
            if json {
                serde_json::to_writer_pretty(&mut out, &result)?;
                writeln!(out)?;
            } else {
                output::write(&mut out, &result)?;
            }
        }
        Command::Complete { ids, json } => {
            if ids.len() == 1 {
                let result = complete::run(&ids[0])?;
                if json {
                    serde_json::to_writer_pretty(&mut out, &result)?;
                    writeln!(out)?;
                } else {
                    output::write_completion(&mut out, &result)?;
                }
            } else {
                let result = complete::batch(&ids, complete::run);
                if !result.is_success() {
                    exit = ExitCode::FAILURE;
                }
                if json {
                    serde_json::to_writer_pretty(&mut out, &result)?;
                    writeln!(out)?;
                } else {
                    output::write_batch(&mut out, &result)?;
                }
            }
        }
    }
    out.flush()?;
    Ok(exit)
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(exit) => exit,
        Err(error)
            if error.chain().any(|cause| {
                cause
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
            }) =>
        {
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
