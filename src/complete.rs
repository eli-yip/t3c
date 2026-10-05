use std::{
    collections::HashSet,
    fmt, fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

use crate::database;

#[derive(Serialize)]
pub struct Completion {
    pub id: String,
    pub title: String,
    pub status: &'static str,
    pub changed: bool,
}

// Marks errors after automation may have sent the write.
#[derive(Debug)]
struct Unconfirmed;

impl fmt::Display for Unconfirmed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("completion could not be confirmed; the to-do may already be completed")
    }
}

impl std::error::Error for Unconfirmed {}

#[derive(Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ItemResult {
    Succeeded {
        #[serde(flatten)]
        completion: Completion,
    },
    Failed {
        id: String,
        error: String,
    },
    Unconfirmed {
        id: String,
        error: String,
    },
}

#[derive(Default, Serialize)]
pub struct Summary {
    pub succeeded: usize,
    pub failed: usize,
    pub unconfirmed: usize,
}

#[derive(Serialize)]
pub struct Batch {
    pub results: Vec<ItemResult>,
    pub summary: Summary,
}

impl Batch {
    pub fn is_success(&self) -> bool {
        self.summary.failed == 0 && self.summary.unconfirmed == 0
    }
}

pub fn batch(ids: &[String], mut run: impl FnMut(&str) -> Result<Completion>) -> Batch {
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    let mut summary = Summary::default();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        results.push(match run(id) {
            Ok(completion) => {
                summary.succeeded += 1;
                ItemResult::Succeeded { completion }
            }
            Err(error) => {
                let message = format!("{error:#}");
                if error.is::<Unconfirmed>() {
                    summary.unconfirmed += 1;
                    ItemResult::Unconfirmed {
                        id: id.clone(),
                        error: message,
                    }
                } else {
                    summary.failed += 1;
                    ItemResult::Failed {
                        id: id.clone(),
                        error: message,
                    }
                }
            }
        });
    }
    Batch { results, summary }
}

pub fn run(id: &str) -> Result<Completion> {
    let path = database::locate()?;
    // A snapshot can contain the same ID as the live database. Never use it to
    // authorize a write to Things or to verify that write.
    let live = database::locate_default()?;
    if fs::canonicalize(&path)? != fs::canonicalize(&live)? {
        bail!("complete requires the local Things database; T3C_DATABASE points elsewhere");
    }
    complete(&path, id, dispatch, inspect, Duration::from_secs(5))
}

fn complete(
    path: &Path,
    id: &str,
    dispatch: impl FnOnce(&str) -> Result<bool>,
    inspect: impl FnOnce(&str) -> Result<bool>,
    verify_timeout: Duration,
) -> Result<Completion> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("cannot open Things database read-only")?;
    connection.busy_timeout(Duration::from_millis(200))?;
    let target = connection
        .query_row(
            r#"SELECT coalesce(t.title, ''), t.type, t.status,
            (coalesce(t.trashed, 0) != 0 OR coalesce(p.trashed, 0) != 0),
            t.rt1_recurrenceRule IS NOT NULL
        FROM TMTask t LEFT JOIN TMTask p ON p.uuid = t.project WHERE t.uuid = ?1"#,
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .optional()
        .context("cannot inspect the Things to-do")?
        .with_context(|| format!("to-do ID not found: {id}"))?;
    let (title, kind, status, trashed, repeating_template) = target;
    if kind != 0 {
        bail!("ID is not a to-do: {id}");
    }
    if trashed {
        bail!("to-do or its project is in the trash: {id}");
    }
    if repeating_template {
        bail!(
            "ID belongs to a repeating template; complete an individual occurrence instead: {id}"
        );
    }
    let changed = match status {
        3 => false,
        2 => bail!("to-do is canceled: {id}"),
        0 => {
            match dispatch(id) {
                Ok(changed) => {
                    verify(&connection, id, verify_timeout).context(Unconfirmed)?;
                    changed
                }
                Err(error) if error.is::<Unconfirmed>() => {
                    // A timeout does not cancel a write already received by Things.
                    // Reconcile the same ID without sending another write.
                    let things_status = inspect(id);
                    verify(&connection, id, verify_timeout)
                        .with_context(|| format!("{error:#}"))
                        .context(Unconfirmed)?;
                    if !things_status
                        .with_context(|| format!("{error:#}"))
                        .context(Unconfirmed)?
                    {
                        return Err(error);
                    }
                    true
                }
                Err(error) => return Err(error),
            }
        }
        value => bail!("unsupported Things task status: {value}"),
    };
    Ok(Completion {
        id: id.to_owned(),
        title,
        status: "completed",
        changed,
    })
}

fn verify(connection: &Connection, id: &str, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        // Each query gets a fresh snapshot, including changes committed to WAL.
        let status: Option<i64> = connection
            .query_row("SELECT status FROM TMTask WHERE uuid = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        if status == Some(3) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for completed status for ID {id}");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn dispatch(id: &str) -> Result<bool> {
    let output = automate(include_str!("complete.applescript"), id)?;
    match output.as_slice().trim_ascii() {
        b"changed" => Ok(true),
        b"unchanged" => Ok(false),
        _ => Err(anyhow::anyhow!("Things returned an unexpected response")).context(Unconfirmed),
    }
}

fn inspect(id: &str) -> Result<bool> {
    let output = automate(include_str!("completion-status.applescript"), id)?;
    Ok(output.as_slice().trim_ascii() == b"completed")
}

fn automate(script: &str, id: &str) -> Result<Vec<u8>> {
    let mut child = Command::new("/usr/bin/osascript")
        .args(["-e", script, "--", id])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot start Things automation")?;
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                if let Err(error) = result {
                    return Err(error)
                        .context("cannot wait for Things automation")
                        .context(Unconfirmed);
                }
                return Err(anyhow::anyhow!("Things automation timed out")).context(Unconfirmed);
            }
        }
    }
    let output = child
        .wait_with_output()
        .context("cannot read Things automation result")
        .context(Unconfirmed)?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        if message.contains("-1743") {
            bail!(
                "macOS denied automation access to Things; allow it in System Settings > Privacy & Security > Automation"
            );
        }
        return Err(anyhow::anyhow!(
            "Things automation failed: {}",
            message.trim()
        ))
        .context(Unconfirmed);
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let connection = Connection::open(dir.path().join("main.sqlite")).unwrap();
        connection.execute_batch(r#"
            PRAGMA journal_mode=WAL;
            CREATE TABLE TMTask(uuid TEXT PRIMARY KEY, title TEXT, type INTEGER DEFAULT 0,
                status INTEGER DEFAULT 0, trashed INTEGER DEFAULT 0, project TEXT, rt1_recurrenceRule BLOB);
            INSERT INTO TMTask(uuid,title) VALUES ('a','Same title'), ('b','Same title');
            INSERT INTO TMTask(uuid,title,status) VALUES ('done','Done',3), ('canceled','Canceled',2);
            INSERT INTO TMTask(uuid,title,type,trashed) VALUES ('project','Project',1,1);
            INSERT INTO TMTask(uuid,title,project) VALUES ('child','Child','project');
            INSERT INTO TMTask(uuid,title,trashed) VALUES ('trash','Trash',1);
            INSERT INTO TMTask(uuid,title,rt1_recurrenceRule) VALUES ('template','Repeating',X'01');
        "#).unwrap();
        (dir, connection)
    }

    #[test]
    fn completes_only_the_requested_id_and_repeated_calls_do_not_dispatch() {
        let (dir, writer) = fixture();
        let path = dir.path().join("main.sqlite");
        let result = complete(
            &path,
            "a",
            |id| {
                assert_eq!(id, "a");
                writer.execute("UPDATE TMTask SET status=3 WHERE uuid=?1", [id])?;
                Ok(true)
            },
            |_| panic!("must not reconcile acknowledged completion"),
            Duration::ZERO,
        )
        .unwrap();
        assert!(result.changed);
        assert_eq!(
            writer
                .query_row("SELECT status FROM TMTask WHERE uuid='b'", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let again = complete(
            &path,
            "a",
            |_| panic!("must not dispatch again"),
            |_| panic!("must not reconcile acknowledged completion"),
            Duration::ZERO,
        )
        .unwrap();
        assert!(!again.changed);
    }

    #[test]
    fn invalid_targets_never_dispatch() {
        let (dir, _writer) = fixture();
        for id in [
            "missing", "canceled", "project", "child", "trash", "template",
        ] {
            assert!(
                complete(
                    &dir.path().join("main.sqlite"),
                    id,
                    |_| panic!("must not dispatch"),
                    |_| panic!("must not reconcile rejected target"),
                    Duration::ZERO
                )
                .is_err()
            );
        }
    }

    #[test]
    fn dispatch_and_verification_failures_never_report_success() {
        let (dir, _writer) = fixture();
        let path = dir.path().join("main.sqlite");
        assert!(
            complete(
                &path,
                "a",
                |_| bail!("permission denied"),
                |_| panic!("must not inspect"),
                Duration::ZERO
            )
            .is_err()
        );
        let unconfirmed = complete(
            &path,
            "a",
            |_| Ok(true),
            |_| panic!("must not inspect"),
            Duration::ZERO,
        )
        .err()
        .unwrap();
        assert!(unconfirmed.to_string().contains("could not be confirmed"));
    }

    #[test]
    fn batch_preserves_order_deduplicates_and_continues_after_failure_and_uncertainty() {
        let (dir, writer) = fixture();
        let ids = ["a", "missing", "a", "done", "b", "canceled"].map(str::to_owned);
        let mut dispatched = Vec::new();
        let result = batch(&ids, |id| {
            complete(
                &dir.path().join("main.sqlite"),
                id,
                |id| {
                    dispatched.push(id.to_owned());
                    if id == "a" {
                        writer.execute("UPDATE TMTask SET status=3 WHERE uuid=?1", [id])?;
                    }
                    Ok(true)
                },
                |_| panic!("acknowledged write"),
                Duration::ZERO,
            )
        });
        assert_eq!(dispatched, ["a", "b"]);
        assert!(!result.is_success());
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(
            json["summary"],
            serde_json::json!({"succeeded":2,"failed":2,"unconfirmed":1})
        );
        assert_eq!(
            json["results"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["a", "missing", "done", "b", "canceled"]
        );
        assert_eq!(json["results"][0]["outcome"], "succeeded");
        assert_eq!(json["results"][1]["outcome"], "failed");
        assert_eq!(json["results"][2]["changed"], false);
        assert_eq!(json["results"][3]["outcome"], "unconfirmed");
    }

    #[test]
    fn batch_completes_each_id_before_starting_the_next() {
        let (dir, writer) = fixture();
        let result = batch(&["a".into(), "b".into()], |id| {
            complete(
                &dir.path().join("main.sqlite"),
                id,
                |id| {
                    if id == "b" {
                        assert_eq!(
                            writer.query_row(
                                "SELECT status FROM TMTask WHERE uuid='a'",
                                [],
                                |r| r.get::<_, i64>(0)
                            )?,
                            3
                        );
                    }
                    writer.execute("UPDATE TMTask SET status=3 WHERE uuid=?1", [id])?;
                    Ok(true)
                },
                |_| panic!("acknowledged write"),
                Duration::ZERO,
            )
        });
        assert!(result.is_success());
        assert_eq!(result.summary.succeeded, 2);
    }

    #[test]
    fn ambiguous_dispatch_reconciles_same_id_without_retrying_the_write() {
        // Both Things and a fresh database snapshot must confirm completion.
        for (database_completed, things_completed) in
            [(true, true), (true, false), (false, true), (false, false)]
        {
            let (dir, writer) = fixture();
            let mut writes = 0;
            let result = complete(
                &dir.path().join("main.sqlite"),
                "a",
                |id| {
                    writes += 1;
                    if database_completed {
                        writer.execute("UPDATE TMTask SET status=3 WHERE uuid=?1", [id])?;
                    }
                    Err(anyhow::anyhow!("automation timed out")).context(Unconfirmed)
                },
                |id| {
                    assert_eq!(id, "a");
                    Ok(things_completed)
                },
                Duration::ZERO,
            );
            assert_eq!(writes, 1);
            if database_completed && things_completed {
                assert!(result.unwrap().changed);
            } else {
                let error = result.err().unwrap();
                assert!(error.is::<Unconfirmed>());
                assert!(format!("{error:#}").contains("automation timed out"));
            }
        }
    }

    #[test]
    fn failed_readback_after_ambiguous_write_remains_unconfirmed() {
        let (dir, writer) = fixture();
        let result = complete(
            &dir.path().join("main.sqlite"),
            "a",
            |id| {
                writer.execute("UPDATE TMTask SET status=3 WHERE uuid=?1", [id])?;
                Err(anyhow::anyhow!("automation timed out")).context(Unconfirmed)
            },
            |_| bail!("readback unavailable"),
            Duration::ZERO,
        );
        assert!(result.err().unwrap().is::<Unconfirmed>());
    }
}
