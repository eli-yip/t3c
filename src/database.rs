use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;

#[derive(Serialize)]
pub struct SearchResult {
    pub schema_version: u8,
    pub query: String,
    pub total: i64,
    pub count: usize,
    pub offset: i64,
    pub limit: Option<i64>,
    pub has_more: bool,
    pub items: Vec<Item>,
}

#[derive(Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub status: &'static str,
    pub trashed: bool,
    pub project: Option<Named>,
    pub area: Option<Named>,
    pub matches: Vec<Match>,
}

#[derive(Serialize)]
pub struct Named {
    pub id: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct Match {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<bool>,
    pub text: String,
}

pub fn locate() -> Result<PathBuf> {
    if let Some(path) = env::var_os("T3C_DATABASE") {
        if path.is_empty() {
            bail!("T3C_DATABASE is empty; set it to a Things database file");
        }
        return Ok(path.into());
    }
    locate_default()
}

pub fn locate_default() -> Result<PathBuf> {
    let home = env::var_os("HOME")
        .context("HOME is unset; set T3C_DATABASE to your Things database file")?;
    let root =
        PathBuf::from(home).join("Library/Group Containers/JLMPQHK86H.com.culturedcode.ThingsMac");
    let mut candidates = Vec::new();
    if root.is_dir() {
        for entry in fs::read_dir(&root).context("cannot search the Things data directory")? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("ThingsData-")
            {
                let path = entry
                    .path()
                    .join("Things Database.thingsdatabase/main.sqlite");
                if path.is_file() {
                    candidates.push(path);
                }
            }
        }
        let legacy = root.join("Things Database.thingsdatabase/main.sqlite");
        if legacy.is_file() {
            candidates.push(legacy);
        }
    }
    candidates.sort();
    match candidates.len() {
        0 => bail!("Things database not found; set T3C_DATABASE to its main.sqlite file"),
        1 => Ok(candidates.remove(0)),
        _ => bail!(
            "multiple Things databases found; select one with T3C_DATABASE:\n{}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

// Each hit is a complete field. EXISTS keeps pagination at the to-do level.
const MATCHED: &str = r#"
WITH hits AS (
    SELECT uuid AS task, 'title' AS field, title AS text,
           NULL AS checklist_id, NULL AS completed, 0 AS field_order, 0 AS position
    FROM TMTask WHERE type = 0 AND instr(lower(title), lower(?1)) > 0
    UNION ALL
    SELECT uuid, 'notes', notes, NULL, NULL, 1, 0
    FROM TMTask WHERE type = 0 AND instr(lower(notes), lower(?1)) > 0
    UNION ALL
    SELECT task, 'checklist', title, uuid, status = 3, 2, "index"
    FROM TMChecklistItem WHERE instr(lower(title), lower(?1)) > 0
      AND (?3 OR status != 3)
), matched AS (
    SELECT t.uuid, coalesce(t.title, '') AS title, t.status,
           (coalesce(t.trashed, 0) != 0 OR coalesce(p.trashed, 0) != 0) AS trashed,
           p.uuid AS project_id, coalesce(p.title, '') AS project_title,
           a.uuid AS area_id, coalesce(a.title, '') AS area_title,
           coalesce(t.userModificationDate, 0) AS modified
    FROM TMTask t
    LEFT JOIN TMTask p ON p.uuid = t.project
    LEFT JOIN TMArea a ON a.uuid = CASE WHEN p.uuid IS NOT NULL THEN p.area ELSE t.area END
    WHERE t.type = 0
      AND (?3 OR t.status != 3)
      AND (?4 OR t.status != 2)
      AND (?2 OR (coalesce(t.trashed, 0) = 0 AND coalesce(p.trashed, 0) = 0))
      AND EXISTS (SELECT 1 FROM hits WHERE hits.task = t.uuid)
)
"#;

pub fn search(
    path: &Path,
    query: &str,
    limit: Option<i64>,
    offset: i64,
    include_trashed: bool,
    include_completed: bool,
    include_canceled: bool,
) -> Result<SearchResult> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("cannot open Things database read-only: {}", path.display()))?;
    connection.busy_timeout(Duration::from_secs(2))?;
    let transaction = connection.transaction()?;
    let total: i64 = transaction
        .query_row(
            &format!("{MATCHED} SELECT count(*) FROM matched"),
            params![query, include_trashed, include_completed, include_canceled],
            |row| row.get(0),
        )
        .context("cannot search Things titles, notes, and checklists")?;
    let sql = format!(
        r#"{MATCHED}, page AS (
        SELECT * FROM matched ORDER BY modified DESC, uuid ASC LIMIT ?5 OFFSET ?6
    )
    SELECT page.*, hits.field, hits.text, hits.checklist_id, hits.completed
    FROM page JOIN hits ON hits.task = page.uuid
    ORDER BY page.modified DESC, page.uuid ASC, hits.field_order, hits.position, hits.checklist_id
    "#
    );
    let mut statement = transaction
        .prepare(&sql)
        .context("cannot prepare Things search results")?;
    let mut rows = statement.query(params![
        query,
        include_trashed,
        include_completed,
        include_canceled,
        limit.unwrap_or(-1),
        offset
    ])?;
    let mut items: Vec<Item> = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        if items.last().is_none_or(|item| item.id != id) {
            let status = match row.get::<_, i64>(2)? {
                0 => "incomplete",
                2 => "canceled",
                3 => "completed",
                value => bail!("unsupported Things task status: {value}"),
            };
            items.push(Item {
                id,
                title: row.get(1)?,
                status,
                trashed: row.get(3)?,
                project: row
                    .get::<_, Option<String>>(4)?
                    .map(|id| -> rusqlite::Result<Named> {
                        Ok(Named {
                            id,
                            title: row.get(5)?,
                        })
                    })
                    .transpose()?,
                area: row
                    .get::<_, Option<String>>(6)?
                    .map(|id| -> rusqlite::Result<Named> {
                        Ok(Named {
                            id,
                            title: row.get(7)?,
                        })
                    })
                    .transpose()?,
                matches: Vec::new(),
            });
        }
        if let Some(item) = items.last_mut() {
            item.matches.push(Match {
                field: row.get(9)?,
                text: row.get(10)?,
                id: row.get(11)?,
                completed: row.get(12)?,
            });
        }
    }
    Ok(SearchResult {
        schema_version: 1,
        query: query.to_owned(),
        total,
        count: items.len(),
        offset,
        limit,
        has_more: total.saturating_sub(offset) > items.len() as i64,
        items,
    })
}
