mod database;
mod output;

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Search Things 3 titles, notes, and checklists")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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

fn run(cli: Cli) -> Result<()> {
    let Command::Search {
        query,
        json,
        limit,
        offset,
        include_trashed,
        include_completed,
        include_canceled,
    } = cli.command;
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
    let mut out = io::BufWriter::new(io::stdout().lock());
    if json {
        serde_json::to_writer_pretty(&mut out, &result)?;
        writeln!(out)?;
    } else {
        output::write(&mut out, &result)?;
    }
    out.flush()?;
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
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
