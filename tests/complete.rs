use rusqlite::Connection;
use serde_json::{Value, json};
use std::process::{Command, Output};
use tempfile::TempDir;

struct Fixture {
    home: TempDir,
    database: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join(
            "Library/Group Containers/JLMPQHK86H.com.culturedcode.ThingsMac/ThingsData-Test/Things Database.thingsdatabase",
        );
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("main.sqlite");
        Connection::open(&database).unwrap().execute_batch(
            "CREATE TABLE TMTask(uuid TEXT PRIMARY KEY, title TEXT, type INTEGER DEFAULT 0,
                status INTEGER DEFAULT 0, trashed INTEGER DEFAULT 0, project TEXT, rt1_recurrenceRule BLOB);
             INSERT INTO TMTask(uuid,title,status) VALUES ('done','Done',3), ('other','Other',3), ('canceled','Canceled',2);",
        ).unwrap();
        Self { home, database }
    }

    // Only completed or invalid IDs exist in this fixture: no AppleScript writes.
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_t3c"));
        command
            .env("HOME", self.home.path())
            .env_remove("T3C_DATABASE");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
}

#[test]
fn single_id_retains_exact_json_human_output_and_error_contract() {
    let fixture = Fixture::new();
    let success = fixture.run(&["complete", "done", "--json"]);
    assert!(success.status.success());
    assert!(success.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&success.stdout).unwrap(),
        json!({
            "id":"done", "title":"Done", "status":"completed", "changed":false
        })
    );
    let human = fixture.run(&["complete", "done"]);
    assert_eq!(
        String::from_utf8(human.stdout).unwrap(),
        "Already completed: Done\nID: done\n"
    );
    let failure = fixture.run(&["complete", "missing", "--json"]);
    assert_eq!(failure.status.code(), Some(1));
    assert!(failure.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("to-do ID not found: missing"));
}

#[test]
fn batch_stdout_is_complete_json_even_with_nonzero_exit() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        "complete", "done", "missing", "done", "canceled", "other", "--json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["summary"],
        json!({"succeeded":2,"failed":2,"unconfirmed":0})
    );
    assert_eq!(
        result["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["done", "missing", "canceled", "other"]
    );
    assert_eq!(result["results"][0]["changed"], false);
    assert_eq!(result["results"][1]["outcome"], "failed");
    assert_eq!(result["results"][3]["outcome"], "succeeded");
}

#[test]
fn all_success_and_duplicate_only_batches_keep_batch_shape() {
    let fixture = Fixture::new();
    for (ids, count) in [(vec!["done", "other"], 2), (vec!["done", "done"], 1)] {
        let mut args = vec!["complete", "--json"];
        args.extend(ids);
        let output = fixture.run(&args);
        assert!(output.status.success());
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["summary"]["succeeded"], count);
        assert_eq!(result["results"].as_array().unwrap().len(), count as usize);
    }
    let human = fixture.run(&["complete", "missing", "done"]);
    assert_eq!(human.status.code(), Some(1));
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains("Failed:") && text.contains("Already completed: Done"));
    assert!(text.contains("1 succeeded, 1 failed, 0 unconfirmed"));
}

#[test]
fn database_copy_is_rejected_for_every_id_even_if_already_completed() {
    let fixture = Fixture::new();
    let copy = fixture.home.path().join("copy.sqlite");
    std::fs::copy(&fixture.database, &copy).unwrap();
    let output = fixture
        .command()
        .env("T3C_DATABASE", copy)
        .args(["complete", "done", "other", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["summary"]["failed"], 2);
    for item in result["results"].as_array().unwrap() {
        assert!(
            item["error"]
                .as_str()
                .unwrap()
                .contains("requires the local Things database")
        );
    }
}
