use std::{
    fs,
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

pub fn run(id: &str) -> Result<Completion> {
    let path = database::locate()?;
    // A snapshot can contain the same ID as the live database. Never use it to
    // authorize a write to Things or to verify that write.
    let live = database::locate_default()?;
    if fs::canonicalize(&path)? != fs::canonicalize(&live)? {
        bail!("complete requires the local Things database; T3C_DATABASE points elsewhere");
    }
    complete(&path, id, dispatch, Duration::from_secs(5))
}

fn complete(
    path: &Path,
    id: &str,
    dispatch: impl FnOnce(&str) -> Result<bool>,
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
            let changed = dispatch(id)?;
            verify(&connection, id, verify_timeout)
                .context("completion was sent, but its result could not be confirmed; the to-do may already be completed")?;
            changed
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
    let mut child = Command::new("/usr/bin/osascript")
        .args(["-e", include_str!("complete.applescript"), "--", id])
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
                        .context("cannot wait for Things automation; completion is unconfirmed");
                }
                bail!(
                    "Things automation timed out; completion is unconfirmed. Check Things before retrying"
                );
            }
        }
    }
    let output = child
        .wait_with_output()
        .context("cannot read Things automation result; completion is unconfirmed")?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        if message.contains("-1743") {
            bail!(
                "macOS denied automation access to Things; allow it in System Settings > Privacy & Security > Automation"
            );
        }
        bail!(
            "Things automation failed; completion is unconfirmed: {}",
            message.trim()
        );
    }
    match output.stdout.as_slice().trim_ascii() {
        b"changed" => Ok(true),
        b"unchanged" => Ok(false),
        _ => bail!("Things returned an unexpected response; completion is unconfirmed"),
    }
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
        assert!(complete(&path, "a", |_| bail!("permission denied"), Duration::ZERO).is_err());
        let unconfirmed = complete(&path, "a", |_| Ok(true), Duration::ZERO)
            .err()
            .unwrap();
        assert!(unconfirmed.to_string().contains("could not be confirmed"));
    }
}
