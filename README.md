# bd-grep

Full-text search over [beads](https://github.com/steveyegge/beads) (`bd`) issues.

`bd-grep` searches every text field of every issue (title, description,
design, acceptance criteria, notes, close reason and comments) and prints one
line per matching issue. The output is plain TSV, so it works with `fzf`,
`cut`, `awk`, or an editor's completion UI.

```console
$ bd-grep -E 'kazoo|seafoam'
zoo-a1	open	Teach the parrot to whistle	comments	Tried the kazoo approach; parrot unimpressed
zoo-b2	closed	Repaint the lighthouse	close_reason	Done in seafoam green
```

## Install

Requires Rust 1.85 or newer, and `bd` on your `PATH` at run time.

```sh
cargo install --git https://github.com/knwoop/bd-grep
```

Or from a checkout:

```sh
cargo install --path .
```

## Usage

```
bd-grep [OPTIONS] PATTERN
```

| Option | Description |
|---|---|
| `-E`, `--regex` | Treat `PATTERN` as a regular expression ([Rust `regex` syntax](https://docs.rs/regex/latest/regex/#syntax)) |
| `-s`, `--status LIST` | Only issues whose status is in `LIST` (comma-separated, e.g. `open,in_progress`). Default: all statuses |
| `-C DIR` | Run `bd` in `DIR` (passed to `bd -C`) |
| `--json` | Print the raw JSON line of each matching issue instead of TSV |
| `--input FILE` | Read JSONL from `FILE` instead of running `bd` (`-` for stdin) |

Matching is always case-insensitive. Without `-E`, `PATTERN` is a fixed
string, so `a.b(c)` matches literally.

### Output

One line per matching issue, in export order:

```
id<TAB>status<TAB>title<TAB>field<TAB>snippet
```

- `field` is the first field that matched, tried in this order: `title`,
  `description`, `design`, `acceptance_criteria`, `notes`, `close_reason`,
  `comments`.
- `snippet` is the match plus about 30 characters on each side. Truncated ends
  are marked with `…`. Tabs and newlines become spaces, and multibyte text
  (Japanese, emoji, …) is never cut in the middle of a character.

### Exit status

Same as `grep`: `0` if an issue matched, `1` if none did, `2` on error (bad
arguments, invalid regex, `bd` failure, malformed JSON).

## Examples

Pick an issue with fzf and show it:

```sh
bd-grep parrot \
  | fzf --delimiter '\t' --with-nth 1,3,5 \
        --preview 'bd --readonly show {1}' \
  | cut -f1 \
  | xargs -r bd show
```

Search as you type (each keystroke runs a fresh export, so this suits small
to medium databases):

```sh
fzf --disabled --delimiter '\t' --with-nth 1,3,5 \
    --bind 'change:reload:bd-grep {q} || true' \
    --preview 'bd --readonly show {1}'
```

Open issues only, as JSON, piped to `jq`:

```sh
bd-grep --json -s open whistle | jq -r '.id + " " + .title'
```

Search an export file without touching the database at all:

```sh
bd --readonly export -o /tmp/issues.jsonl
bd-grep --input /tmp/issues.jsonl lighthouse
```

## How it works

Each run calls `bd [-C DIR] --readonly export`, which writes every issue to
stdout as JSONL (about a second for a thousand issues), and matches against
it. There is no cache, so results are never stale. `bd-grep` never writes to
`.beads/`, and it always passes `--readonly` to `bd`.

Like `bd export`, it skips infrastructure beads (agents, roles, messages) and
memories. If you pipe in an `--all` export via `--input`, records whose
`_type` is not `issue` are ignored.

## Why not `bd search` or `dolt sql`?

- **`bd search`** matches titles only. `--desc-contains` and
  `--notes-contains` exist, but they are AND-ed together, so you cannot ask
  "does this word appear anywhere in the issue?"
- **`bd list --json`** omits `design`, `notes` and `acceptance_criteria`.
- **`bd sql`** is not supported when beads uses its embedded Dolt database.
- **`dolt sql` on `.beads/embeddeddolt`** reads the database directly, but it
  holds the `.dolt/noms/LOCK` file while it runs, which blocks `bd` (even
  read-only commands), and it creates a `.dolt/stats/` directory as a side
  effect. A search tool should not do either.

`bd --readonly export` is the supported, side-effect-free way to read
everything, so `bd-grep` builds on it.

## Development

```sh
cargo test                 # unit + end-to-end tests (fictional fixtures, fake bd)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The end-to-end tests never run the real `bd`: they use
`tests/fixtures/issues.jsonl` and a stub `bd` script that records its
arguments, which also checks that `--readonly` is always passed.

## License

[MIT](LICENSE)
