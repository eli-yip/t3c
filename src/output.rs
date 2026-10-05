use crate::database::SearchResult;
use std::io::{self, Write};

pub fn write(out: &mut impl Write, result: &SearchResult) -> io::Result<()> {
    writeln!(
        out,
        "{} matching to-dos; showing {} (offset {})",
        result.total, result.count, result.offset
    )?;
    for item in &result.items {
        writeln!(
            out,
            "\n{} [{}{}]\n  id: {}",
            visible(&item.title),
            item.status,
            if item.trashed { ", trashed" } else { "" },
            visible(&item.id)
        )?;
        if let Some(project) = &item.project {
            writeln!(out, "  project: {}", visible(&project.title))?;
        }
        if let Some(area) = &item.area {
            writeln!(out, "  area: {}", visible(&area.title))?;
        }
        for hit in &item.matches {
            let check = match hit.completed {
                Some(true) => " [x]",
                Some(false) => " [ ]",
                None => "",
            };
            writeln!(
                out,
                "  {}{}: {}",
                hit.field,
                check,
                excerpt(&hit.text, &result.query)
            )?;
        }
    }
    Ok(())
}

// Keep terminal control characters inert. JSON retains the original text.
pub fn visible(text: &str) -> String {
    let mut result = String::new();
    for character in text.chars() {
        if character.is_control() {
            result.extend(character.escape_default());
        } else {
            result.push(character);
        }
    }
    result
}

fn excerpt(text: &str, query: &str) -> String {
    let byte = text
        .to_ascii_lowercase()
        .find(&query.to_ascii_lowercase())
        .unwrap_or(0);
    let start = text[..byte].chars().count().saturating_sub(40);
    let length = 160.max(query.chars().count() + 80);
    let fragment: String = text.chars().skip(start).take(length).collect();
    let end = start + fragment.chars().count();
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        visible(&fragment),
        if text.chars().count() > end {
            "…"
        } else {
            ""
        }
    )
}

pub fn write_completion(
    out: &mut impl Write,
    result: &crate::complete::Completion,
) -> io::Result<()> {
    writeln!(
        out,
        "{}: {}\nID: {}",
        if result.changed {
            "Completed"
        } else {
            "Already completed"
        },
        visible(&result.title),
        visible(&result.id)
    )
}

pub fn write_batch(out: &mut impl Write, batch: &crate::complete::Batch) -> io::Result<()> {
    use crate::complete::ItemResult;
    for item in &batch.results {
        match item {
            ItemResult::Succeeded { completion } => write_completion(out, completion)?,
            ItemResult::Failed { id, error } => {
                writeln!(out, "Failed: {}\nID: {}", visible(error), visible(id))?
            }
            ItemResult::Unconfirmed { id, error } => {
                writeln!(out, "Unconfirmed: {}\nID: {}", visible(error), visible(id))?
            }
        }
    }
    writeln!(
        out,
        "{} succeeded, {} failed, {} unconfirmed",
        batch.summary.succeeded, batch.summary.failed, batch.summary.unconfirmed
    )
}
