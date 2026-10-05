use rusqlite::Connection;
use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    connection: Connection,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let connection = Connection::open(directory.path().join("main.sqlite")).unwrap();
        // Keep the writer open so the child process must read uncheckpointed WAL data.
        connection.execute_batch(r#"
            PRAGMA journal_mode=WAL;
            PRAGMA wal_autocheckpoint=0;
            CREATE TABLE TMTask(uuid TEXT PRIMARY KEY, title TEXT, notes TEXT, type INTEGER DEFAULT 0,
                status INTEGER DEFAULT 0, trashed INTEGER DEFAULT 0, project TEXT, area TEXT, userModificationDate REAL);
            CREATE TABLE TMChecklistItem(uuid TEXT PRIMARY KEY, task TEXT, title TEXT, status INTEGER, "index" INTEGER);
            CREATE TABLE TMArea(uuid TEXT PRIMARY KEY, title TEXT);
            INSERT INTO TMArea VALUES ('area', 'Research');
            INSERT INTO TMTask(uuid,title,type,area) VALUES ('project','weibo project only',1,'area');
            INSERT INTO TMTask(uuid,title,notes,userModificationDate) VALUES ('a','WeIbO title','正文 weibo',30);
            INSERT INTO TMTask(uuid,title,notes,status,project,userModificationDate) VALUES ('b','Checklist only','',3,'project',20);
            INSERT INTO TMTask(uuid,title,notes,status,userModificationDate) VALUES ('c','Notes only','https://weibo.com/中文',2,20);
            INSERT INTO TMTask(uuid,title,notes,userModificationDate) VALUES ('literal','100%_ * 中文','quote '' and weekly review',10);
            INSERT INTO TMTask(uuid,title,trashed,userModificationDate) VALUES ('trash','weibo',1,50);
            INSERT INTO TMTask(uuid,title,type,trashed) VALUES ('dead-project','Deleted',1,1);
            INSERT INTO TMTask(uuid,title,project,userModificationDate) VALUES ('child','weibo','dead-project',40);
            INSERT INTO TMChecklistItem VALUES ('check-a','a','weibo again',0,0), ('check-b','b','Read WEIBO',3,0);
        "#).unwrap();
        Self {
            directory,
            connection,
        }
    }

    fn path(&self) -> std::path::PathBuf {
        self.directory.path().join("main.sqlite")
    }
    fn run(&self, args: &[&str]) -> Output {
        run(&self.path(), args)
    }
    fn json(&self, args: &[&str]) -> Value {
        let mut arguments = vec!["search"];
        arguments.extend_from_slice(args);
        arguments.push("--json");
        let output = self.run(&arguments);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn run(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_t3c"))
        .env("T3C_DATABASE", path)
        .args(args)
        .output()
        .unwrap()
}

fn ids(result: &Value) -> Vec<&str> {
    result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect()
}

#[test]
fn searches_all_fields_and_statuses_from_live_wal_without_duplicate_todos() {
    let fixture = Fixture::new();
    let long_note = format!("{} weibo\nfull text", "前文".repeat(200));
    fixture
        .connection
        .execute("UPDATE TMTask SET notes=?1 WHERE uuid='a'", [&long_note])
        .unwrap();
    let default = fixture.json(&["weibo"]);
    assert_eq!(ids(&default), ["a"]);
    assert_eq!(default["total"], 1);
    let result = fixture.json(&["weibo", "--include-completed", "--include-canceled"]);
    assert_eq!(ids(&result), ["a", "b", "c"]);
    assert_eq!(result["total"], 3);
    assert_eq!(result["count"], 3);
    assert_eq!(result["limit"], Value::Null);
    assert_eq!(result["items"][0]["matches"].as_array().unwrap().len(), 3);
    assert_eq!(result["items"][0]["matches"][1]["text"], long_note);
    assert_eq!(result["items"][0]["matches"][2]["completed"], false);
    assert_eq!(result["items"][1]["matches"][0]["completed"], true);
    assert_eq!(result["items"][1]["status"], "completed");
    assert_eq!(result["items"][2]["status"], "canceled");
    assert_eq!(result["items"][1]["area"]["id"], "area");
    let human = fixture.run(&["search", "weibo"]);
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("weibo") && human.contains('…'));
    assert!(!human.contains(&long_note));
}

#[test]
fn pagination_counts_todos_and_respects_parent_trash() {
    let fixture = Fixture::new();
    let page = fixture.json(&[
        "weibo",
        "--include-completed",
        "--include-canceled",
        "--limit",
        "1",
        "--offset",
        "1",
    ]);
    assert_eq!(ids(&page), ["b"]);
    assert_eq!(page["total"], 3);
    assert_eq!(page["has_more"], true);
    let empty = fixture.json(&[
        "weibo",
        "--include-completed",
        "--include-canceled",
        "--offset",
        "99",
    ]);
    assert_eq!(empty["count"], 0);
    assert_eq!(empty["total"], 3);
    assert_eq!(empty["has_more"], false);
    let all = fixture.json(&["weibo", "--include-trashed"]);
    assert_eq!(ids(&all), ["trash", "child", "a"]);
    assert_eq!(all["items"][1]["trashed"], true);
}

#[test]
fn treats_keywords_as_literal_text_including_unicode_and_sql_characters() {
    let fixture = Fixture::new();
    for query in ["%_", "*", "quote '", "weekly review"] {
        assert_eq!(ids(&fixture.json(&[query])), ["literal"]);
    }
    assert_eq!(ids(&fixture.json(&["中文"])), ["literal"]);
    assert_eq!(fixture.json(&["' OR 1=1 --"])["count"], 0);
    assert_eq!(fixture.json(&["absent"])["total"], 0);
}

#[test]
fn errors_are_nonzero_and_never_return_partial_results_or_create_a_database() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run(&["search", " "]).status.code(), Some(2));
    let missing = fixture.directory.path().join("missing.sqlite");
    assert_eq!(run(&missing, &["search", "weibo"]).status.code(), Some(1));
    assert!(!missing.exists());
    fixture
        .connection
        .execute_batch("DROP TABLE TMChecklistItem")
        .unwrap();
    let failed = fixture.run(&["search", "weibo", "--json"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(failed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("TMChecklistItem"));
}

#[test]
fn discovers_one_database_and_requires_selection_when_ambiguous() {
    let fixture = Fixture::new();
    let home = tempfile::tempdir().unwrap();
    let root = home
        .path()
        .join("Library/Group Containers/JLMPQHK86H.com.culturedcode.ThingsMac");
    let first = root.join("ThingsData-A/Things Database.thingsdatabase");
    std::fs::create_dir_all(&first).unwrap();
    fixture
        .connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    std::fs::copy(fixture.path(), first.join("main.sqlite")).unwrap();
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_t3c"))
            .env_remove("T3C_DATABASE")
            .env("HOME", home.path())
            .args(["search", "weibo", "--json"])
            .output()
            .unwrap()
    };
    assert!(invoke().status.success());
    let second = root.join("ThingsData-B/Things Database.thingsdatabase");
    std::fs::create_dir_all(&second).unwrap();
    std::fs::copy(first.join("main.sqlite"), second.join("main.sqlite")).unwrap();
    let ambiguous = invoke();
    assert_eq!(ambiguous.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("multiple Things databases"));
}

#[test]
fn completion_and_cancellation_flags_are_independent_and_filter_checklist_hits() {
    let fixture = Fixture::new();
    fixture
        .connection
        .execute_batch(
            r#"
        INSERT INTO TMTask(uuid,title,userModificationDate) VALUES ('checked-only','Saved link',15);
        INSERT INTO TMChecklistItem VALUES ('checked','checked-only','weibo',3,0),
            ('checked-title','a','weibo done',3,1);
    "#,
        )
        .unwrap();
    let default = fixture.json(&["weibo"]);
    assert_eq!(ids(&default), ["a"]);
    assert_eq!(default["items"][0]["matches"].as_array().unwrap().len(), 3);
    assert_eq!(
        ids(&fixture.json(&["weibo", "--include-canceled"])),
        ["a", "c"]
    );
    let completed = fixture.json(&["weibo", "--include-completed"]);
    assert_eq!(ids(&completed), ["a", "b", "checked-only"]);
    assert_eq!(
        completed["items"][0]["matches"].as_array().unwrap().len(),
        4
    );
    assert_eq!(completed["total"], 3);
    let page = fixture.json(&["weibo", "--limit", "1", "--offset", "1"]);
    assert_eq!(page["total"], 1);
    assert_eq!(page["count"], 0);
}
