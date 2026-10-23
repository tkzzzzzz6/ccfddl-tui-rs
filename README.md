# CCF-Deadlines TUI (Rust)

A terminal UI for tracking CCF-ranked conference deadlines with live countdowns.

This is a Rust port of the Python/Textual app in [`d:\develop\tui`](../tui). The
data layer is kept behaviour-compatible with the Python original and is verified
against it by `tests/golden.rs`, which compares rows, sort order, fuzzy search
and duration formatting against a snapshot generated from the Python code.

## Status

| Area | State |
|------|-------|
| Data model, timezone parsing, row building | ported, golden-verified |
| Filtering, fuzzy search, sorting | ported, golden-verified |
| Favourites (`~/.ccfddl/favorites.json`) | ported, shared file format |
| Remote / cache / embedded fallback | ported (embedded snapshot is extra) |
| Terminal UI | ported (ratatui) |

## Usage

```bash
cargo run --release                 # start the TUI
cargo run --release -- --list       # print the table and exit
cargo run --release -- --url URL    # custom data source
```

## Keyboard

### Table (default focus)

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `g` / `G` | First / last row |
| `f` | Toggle favourite |
| `l` | Toggle language (EN/ZH) |
| `r` | Refresh from the data source |
| `/` | Search |
| `Tab` | Focus the filter sidebar |
| `Esc` | Reset all filters |
| `?` | Help |
| `q` | Quit |

### Sidebar

| Key | Action |
|-----|--------|
| `↓` / `↑` | Move the cursor between checkboxes |
| `Space` / `Enter` | Toggle the checkbox under the cursor |
| `/` | Search |
| `Tab` | Back to the table |
| `Esc` | Reset all filters |

### Search

| Key | Action |
|-----|--------|
| any character | Type into the query |
| `Backspace` | Delete |
| `Enter` | Apply and return to the table |
| `Esc` | Clear and return to the table |

Search is a separate focus mode so that `q`, `r` and `?` reach the text field
instead of triggering their global shortcuts.

## Intentional differences from the Python app

- **The cursor follows a re-sorted row.** Favouriting moves a row to the top of
  the list; the cursor follows that row instead of staying on the old index,
  which would otherwise select a different conference.
- **Rows expire on time.** A one-second tick flips `is_expired` when a deadline
  passes. Python leaves it frozen until the next reload, so finished conferences
  linger on screen for the rest of the session.
- **`PT` is resolved.** Rust maps `PT` to `America/Los_Angeles` with DST; Python
  rejects it and turns those deadlines into `N/A`. `tests/golden.rs` asserts this
  divergence rather than hiding it.
- **An embedded snapshot** is compiled into the binary via `assets/allconf.yml`
  so the app works offline on first run.

## Development

```bash
cargo test                 # 141 tests, including golden parity
cargo clippy --all-targets
cargo run --example snapshot          # print one rendered frame
cargo run --example snapshot -- --help   # ... with the help overlay
```

`tests/fixtures/gen_golden.py` regenerates `tests/fixtures/golden.json` from the
Python implementation; re-run it only when the Python behaviour itself changes.