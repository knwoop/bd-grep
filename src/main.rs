//! bd-grep - full-text search over beads (bd) issues.
//!
//! Reads `bd --readonly export` (JSONL, one issue per line) and prints one
//! TSV line per matching issue: id, status, title, field, snippet.
//! Exit status follows grep: 0 = match, 1 = no match, 2 = error.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitCode, Stdio};

use bd_grep::{build_regex, find_hit, flatten, is_issue, str_field};
use regex::Regex;
use serde_json::Value;

const USAGE: &str = "\
Usage: bd-grep [OPTIONS] PATTERN

Search title, description, design, acceptance_criteria, notes,
close_reason and comments of beads issues. PATTERN is a case-insensitive
fixed string unless -E is given.

Options:
  -E, --regex           treat PATTERN as a regular expression
  -s, --status LIST     only issues whose status is in LIST (comma-separated)
  -C DIR                run bd in DIR (passed to bd as -C DIR)
      --json            print the matching issues' raw JSON lines
      --input FILE      read JSONL from FILE instead of running bd ('-' = stdin)
  -h, --help            show this help
  -V, --version         show version

Output (default): id<TAB>status<TAB>title<TAB>field<TAB>snippet
Exit status: 0 if an issue matched, 1 if none, 2 on error.
";

#[derive(Debug, Default)]
struct Args {
    pattern: String,
    regex: bool,
    json: bool,
    statuses: Vec<String>,
    dir: Option<PathBuf>,
    input: Option<PathBuf>,
}

enum Parsed {
    Run(Args),
    Exit(&'static str),
}

/// Error carrying a message for stderr; every error exits with status 2.
struct Fail(String);

impl<E: std::fmt::Display> From<E> for Fail {
    fn from(e: E) -> Self {
        Fail(e.to_string())
    }
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Run(args)) => args,
        Ok(Parsed::Exit(text)) => {
            print!("{text}");
            return ExitCode::SUCCESS;
        }
        Err(Fail(msg)) => {
            eprintln!("bd-grep: {msg}");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(Fail(msg)) => {
            eprintln!("bd-grep: {msg}");
            ExitCode::from(2)
        }
    }
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Parsed, Fail> {
    let mut args = Args::default();
    let mut positional = Vec::new();

    fn value(opt: &str, it: &mut impl Iterator<Item = String>) -> Result<String, Fail> {
        it.next()
            .ok_or_else(|| Fail(format!("option '{opt}' requires an argument")))
    }

    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_owned(), Some(v.to_owned())),
            _ => (arg.clone(), None),
        };
        let mut val = |opt: &str| match inline.clone() {
            Some(v) => Ok(v),
            None => value(opt, &mut it),
        };
        match name.as_str() {
            "-E" | "--regex" => args.regex = true,
            "--json" => args.json = true,
            "-s" | "--status" => {
                let list = val(&name)?;
                args.statuses
                    .extend(list.split(',').filter(|s| !s.is_empty()).map(str::to_owned));
            }
            "-C" => args.dir = Some(val(&name)?.into()),
            "--input" => args.input = Some(val(&name)?.into()),
            "-h" | "--help" => return Ok(Parsed::Exit(USAGE)),
            "-V" | "--version" => {
                return Ok(Parsed::Exit(concat!(
                    "bd-grep ",
                    env!("CARGO_PKG_VERSION"),
                    "\n"
                )));
            }
            "--" => {
                positional.extend(it.by_ref());
                break;
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(Fail(format!("unknown option: {arg}")));
            }
            _ => positional.push(arg),
        }
    }

    match <[String; 1]>::try_from(positional) {
        Ok([pattern]) => {
            args.pattern = pattern;
            Ok(Parsed::Run(args))
        }
        Err(_) => Err(Fail("expected exactly one PATTERN".into())),
    }
}

/// Searches every issue and prints hits. Returns whether anything matched.
fn run(args: &Args) -> Result<bool, Fail> {
    let re = build_regex(&args.pattern, args.regex)?;

    let mut child = None;
    let reader: Box<dyn BufRead> = match &args.input {
        Some(p) if p.as_os_str() == "-" => Box::new(io::stdin().lock()),
        Some(p) => Box::new(BufReader::new(
            File::open(p).map_err(|e| Fail(format!("{}: {e}", p.display())))?,
        )),
        None => {
            let mut c = spawn_bd(args.dir.as_ref())?;
            let out = c.stdout.take().expect("stdout is piped");
            child = Some(c);
            Box::new(BufReader::new(out))
        }
    };

    let mut out = BufWriter::new(io::stdout().lock());
    let result = search(reader, args, &re, &mut out).and_then(|found| {
        out.flush()?;
        Ok(found)
    });

    match (result, child) {
        // The reader (e.g. fzf or head) went away: stop quietly, like grep.
        (Err(SearchError::Io(e)), c) if e.kind() == io::ErrorKind::BrokenPipe => {
            if let Some(mut c) = c {
                let _ = c.kill();
                let _ = c.wait();
            }
            Ok(true)
        }
        (Err(e), c) => {
            if let Some(mut c) = c {
                let _ = c.kill();
                let _ = c.wait();
            }
            Err(Fail(e.to_string()))
        }
        (Ok(found), Some(mut c)) => {
            let status = c.wait()?;
            if !status.success() {
                return Err(Fail(format!("bd export failed ({status})")));
            }
            Ok(found)
        }
        (Ok(found), None) => Ok(found),
    }
}

/// Runs `bd [-C DIR] --readonly export`. `--readonly` is not optional:
/// bd-grep must never write to `.beads/`.
fn spawn_bd(dir: Option<&PathBuf>) -> Result<Child, Fail> {
    let mut cmd = Command::new("bd");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    cmd.args(["--readonly", "export"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    cmd.spawn().map_err(|e| Fail(format!("cannot run bd: {e}")))
}

enum SearchError {
    Io(io::Error),
    Json(usize, serde_json::Error),
}

impl From<io::Error> for SearchError {
    fn from(e: io::Error) -> Self {
        SearchError::Io(e)
    }
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::Io(e) => write!(f, "{e}"),
            SearchError::Json(line, e) => write!(f, "line {line}: invalid JSON: {e}"),
        }
    }
}

fn search(
    reader: impl BufRead,
    args: &Args,
    re: &Regex,
    out: &mut impl Write,
) -> Result<bool, SearchError> {
    let mut found = false;
    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let issue: Value = serde_json::from_str(&line).map_err(|e| SearchError::Json(n + 1, e))?;
        if !is_issue(&issue) {
            continue;
        }
        let status = str_field(&issue, "status");
        if !args.statuses.is_empty() && !args.statuses.iter().any(|s| s == status) {
            continue;
        }
        let Some(hit) = find_hit(&issue, re) else {
            continue;
        };
        found = true;
        if args.json {
            writeln!(out, "{line}")?;
        } else {
            writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}",
                flatten(str_field(&issue, "id")),
                flatten(status),
                flatten(str_field(&issue, "title")),
                hit.field,
                hit.snippet,
            )?;
        }
    }
    Ok(found)
}
