# Project Agent Guidelines

## Purpose

CCF-Deadlines TUI is a cross-platform Rust terminal application. Preserve the
existing ratatui/crossterm architecture and the Python-compatible data layer.

## Architecture

- `src/data/`, `src/model.rs`, `src/format.rs`, and `src/timezone.rs` contain
  conference data, parsing, filtering, searching, and formatting.
- `src/ui/app.rs` owns application behavior and selection/filter state.
- `src/ui/view.rs` owns persistent table/sidebar scrolling and mouse hit regions.
- `src/ui/render.rs` renders the current frame and produces hit regions.
- `src/ui/input.rs` routes keyboard and mouse input through the same `App`
  methods. Do not duplicate business logic for different input methods.
- `src/ui/refresh.rs` performs network loading off the UI thread.
- `src/agent/` is the conference recommendation agent and must not depend on
  `src/ui`: `recommend.rs` (offline rule scoring), `topics.rs` (interest and
  venue topics), `profile.rs` (`~/.ccfddl/profile.yaml`), `llm.rs`
  (OpenAI-compatible client behind the `LlmClient` trait), `tools.rs`,
  `session.rs` (tool loop) and `worker.rs` (background thread).
- `src/ui/agent.rs` owns agent panel behaviour and slash commands;
  `src/ui/agent_view.rs` draws it. Facts shown on cards come from the data
  layer, never from model text. Tests inject a scripted `LlmClient` and never
  call a real model.

Keep terminal coordinates out of the data layer. Mouse handlers must consume
the latest rendered `Rect` hit map and the current scroll offset, not hard-coded
positions. Always restore the terminal and disable mouse capture on exit.

## Required validation

Run these commands after code changes, matching `.github/workflows/ci.yml`:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
```

Use `cargo run --example snapshot` for a terminal-independent rendering check.
Run the full suite before committing UI or data-layer changes.

## Conventions

- Use stable Rust and the existing edition. Avoid new dependencies unless the
  task requires them; discuss architectural changes before adding frameworks.
- Keep changes focused. Update tests and `README.md` when behavior, key bindings,
  installation, or workflow behavior changes.
- Do not commit `target/`, release packages, credentials, editor state, or
  machine-specific files.
- Tests must not require a display, a real browser, or live network access. Use
  `TestBackend`, temporary directories, and injected fetchers. Do not open a
  browser during tests.
- Keep mouse and keyboard behavior consistent, including selection identity
  after sorting or favoriting.
- Support Windows, Linux, and macOS. Use platform-specific process execution
  without invoking a shell for downloaded URLs.

## Python parity and golden fixtures

`tests/golden.rs` and `tests/fixtures/golden.json` verify compatibility with the
Python implementation. Do not edit `golden.json` manually. Regenerate it with
`tests/fixtures/gen_golden.py` only when the Python reference behavior changes.

Intentional differences from Python must remain documented in `README.md`.
These currently include search relevance ordering, automatic expiration,
cursor tracking, embedded offline data, and `PT` timezone handling.

## Collaboration and releases

Read `CONTRIBUTING.md` for the contribution workflow. CI changes must preserve
read-only permissions by default. Release changes must keep `contents: write`
limited to the final release job. Do not create tags, publish releases, or push
to a remote unless explicitly requested.

## License

The project uses the MIT License in `LICENSE`. Preserve the existing CCFDDL
copyright notice and document significant third-party changes when appropriate.
