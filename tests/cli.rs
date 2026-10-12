//! End-to-end tests for the bd-grep binary. All data is fictional
//! (tests/fixtures/issues.jsonl); `bd` is replaced by a fake script.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_bd-grep");
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/issues.jsonl");

fn grep(args: &[&str]) -> Output {
    let mut all = vec!["--input", FIXTURE];
    all.extend_from_slice(args);
    Command::new(BIN).args(all).output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn lines(o: &Output) -> Vec<Vec<String>> {
    stdout(o)
        .lines()
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

/// Creates a directory holding a fake `bd` that records its arguments to
/// `args.log` and runs `body`.
fn fake_bd(name: &str, body: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("bd");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}/args.log'\n{body}\n",
            dir.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn with_path(dir: &Path) -> Command {
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap());
    let mut cmd = Command::new(BIN);
    cmd.env("PATH", path);
    cmd
}

#[test]
fn tsv_columns_and_first_field() {
    let o = grep(&["PARROT"]);
    assert_eq!(o.status.code(), Some(0));
    let rows = lines(&o);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0][..4],
        ["zoo-a1", "open", "Teach the parrot to whistle", "title"]
    );
    assert_eq!(rows[0].len(), 5);
}

#[test]
fn snippet_is_one_line_with_context() {
    let o = grep(&["third one"]);
    let rows = lines(&o);
    assert_eq!(rows[0][3], "description");
    assert_eq!(
        rows[0][4],
        "…t knows two notes. It needs a third one before the spring festival."
    );
}

#[test]
fn searches_comments_close_reason_and_other_fields() {
    let field = |p: &str| lines(&grep(&[p]))[0][3].clone();
    assert_eq!(field("kazoo"), "comments");
    assert_eq!(field("seafoam"), "close_reason");
    assert_eq!(field("sunflower"), "design");
    assert_eq!(field("full tune"), "acceptance_criteria");
    assert_eq!(field("mornings"), "notes");
}

#[test]
fn japanese_snippet_is_not_garbled() {
    let o = grep(&["共有カレンダー"]);
    let rows = lines(&o);
    assert_eq!(rows[0][0], "zoo-c3");
    assert_eq!(
        rows[0][4],
        "…回す。担当者が休みのときは代理を立てる。Excel ではなく共有カレンダーで管理する方針。"
    );
}

#[test]
fn fixed_string_by_default_regex_with_e() {
    assert_eq!(lines(&grep(&["a.b(c)*"]))[0][0], "zoo-d4");
    assert_eq!(grep(&["a.b(c)x"]).status.code(), Some(1));
    let o = grep(&["-E", "sea(foam|weed)"]);
    assert_eq!(lines(&o)[0][0], "zoo-b2");
}

#[test]
fn status_filter() {
    let o = grep(&["-s", "closed,in_progress", "e"]);
    let ids: Vec<_> = lines(&o).into_iter().map(|r| r[0].clone()).collect();
    assert_eq!(ids, ["zoo-b2", "zoo-c3"]);
    assert_eq!(grep(&["--status=closed", "parrot"]).status.code(), Some(1));
}

#[test]
fn non_issue_records_are_ignored() {
    let o = grep(&["otter"]);
    let ids: Vec<_> = lines(&o).into_iter().map(|r| r[0].clone()).collect();
    assert_eq!(ids, ["zoo-d4"]);
}

#[test]
fn json_prints_raw_lines() {
    let o = grep(&["--json", "lighthouse"]);
    let raw = fs::read_to_string(FIXTURE).unwrap();
    let expected = raw.lines().find(|l| l.contains("zoo-b2")).unwrap();
    assert_eq!(stdout(&o), format!("{expected}\n"));
}

#[test]
fn reads_stdin() {
    let mut child = Command::new(BIN)
        .args(["--input", "-", "kazoo"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&fs::read(FIXTURE).unwrap())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(lines(&o)[0][0], "zoo-a1");
}

#[test]
fn exit_codes() {
    assert_eq!(grep(&["no such text anywhere"]).status.code(), Some(1));
    assert_eq!(grep(&["-E", "("]).status.code(), Some(2));
    assert_eq!(grep(&[]).status.code(), Some(2));
    assert_eq!(grep(&["--bogus", "x"]).status.code(), Some(2));
    let o = Command::new(BIN)
        .args(["--input", "/nonexistent/file", "x"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn invalid_json_is_an_error() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let bad = dir.join("bad.jsonl");
    fs::write(&bad, "{\"id\":\"x\"}\nnot json\n").unwrap();
    let o = Command::new(BIN)
        .args(["--input", bad.to_str().unwrap(), "x"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("line 2"));
}

#[test]
fn runs_bd_readonly_export_with_dir() {
    let dir = fake_bd("fake-bd-ok", &format!("cat '{FIXTURE}'"));
    let o = with_path(&dir)
        .args(["-C", "/some/project", "lighthouse"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(lines(&o)[0][0], "zoo-b2");
    let args = fs::read_to_string(dir.join("args.log")).unwrap();
    assert_eq!(args, "-C\n/some/project\n--readonly\nexport\n");
}

#[test]
fn runs_bd_without_dir() {
    let dir = fake_bd("fake-bd-nodir", &format!("cat '{FIXTURE}'"));
    let o = with_path(&dir).arg("lighthouse").output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let args = fs::read_to_string(dir.join("args.log")).unwrap();
    assert_eq!(args, "--readonly\nexport\n");
}

#[test]
fn bd_failure_is_exit_2() {
    let dir = fake_bd("fake-bd-fail", "echo 'boom' >&2; exit 3");
    let o = with_path(&dir).arg("x").output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("bd export failed"));
}
