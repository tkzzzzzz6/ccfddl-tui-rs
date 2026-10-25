# Contributing to CCF-Deadlines TUI

Thank you for helping improve the Rust port. Contributions should be small,
reviewable, tested, and compatible with Windows, Linux, and macOS.

## Development setup

Install a current stable Rust toolchain with `rustfmt` and `clippy`, then run:

```bash
cargo build
cargo run -- --list
cargo run --example snapshot
```

The test suite does not require Python, a display, a real browser, or network
access. Python is needed only when regenerating fixtures from the reference
implementation.

## Before making changes

- Open an issue for substantial features, dependency changes, or architectural
  work. Include the problem and proposed behavior rather than only a solution.
- For bug reports, include the operating system, terminal, application version,
  reproduction steps, expected behavior, and actual behavior. Screenshots are
  useful for TUI issues. Do not include credentials or personal data.
- Create a focused branch and keep unrelated formatting or refactoring out of
  feature and bug-fix changes.

## Implementation guidelines

- Preserve the separation between application behavior (`App`), persistent view
  state (`ViewState`), rendering, input routing, and the data layer.
- Keyboard and mouse input must call the same application methods. Mouse input
  must use the latest rendered hit regions and scroll offsets.
- Keep terminal restoration and mouse-capture cleanup reliable on normal exit
  and error paths.
- Avoid new dependencies when the existing standard library and crates provide
  a suitable solution.
- Do not invoke a shell to open downloaded URLs.
- Do not commit `target/`, generated packages, local configuration, secrets, or
  editor-specific files.

See `AGENTS.md` for architecture-specific agent and maintainer conventions.

## Tests and checks

Run the same checks as CI before submitting:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
```

Add or update tests for every behavioral change. Relevant suites include:

- Unit tests beside source modules;
- Python parity tests in `tests/golden.rs`;
- Coordinate-level mouse and responsive-layout tests in `tests/ui_mouse.rs`;
- Terminal-independent rendering through `examples/snapshot.rs`.

Do not write tests that open a browser or depend on live network access. Use
injected fetchers and temporary directories instead.

### Golden fixtures

Do not manually edit `tests/fixtures/golden.json`. Regenerate it with
`tests/fixtures/gen_golden.py` only when the Python reference behavior changes.
Document intentional differences in `README.md` instead of hiding them in test
expectations.

## Documentation

Update `README.md` when changing user-visible behavior, shortcuts, installation,
CI, or release processes. Update `AGENTS.md` when architecture or maintainer
conventions change. Comments should explain non-obvious decisions rather than
restate the code.

## Commits and pull requests

Use concise Conventional Commit subjects, for example:

```text
feat(ui): add keyboard shortcuts for details
fix(search): preserve exact-match relevance
test(mouse): cover scrolling after resize
ci: validate release packages
```

A pull request should include:

- The problem being solved and the chosen approach;
- Relevant behavior or compatibility notes;
- The exact validation commands and their results;
- Screenshots or snapshots for user-visible TUI changes;
- Linked issues where applicable.

Do not push tags, rewrite public history, or publish releases as part of an
ordinary feature PR. Reviewers may request platform-specific verification for
terminal, mouse, packaging, or process-execution changes.

## CI and releases

Pull requests and branch pushes run `.github/workflows/ci.yml`. Release builds
use `.github/workflows/release.yml` and are intentionally separate.

Manual Release-workflow runs produce test artifacts only. A `v*.*.*` tag must
match `Cargo.toml`; it creates a public GitHub Release after validation and native
Windows/Linux/macOS builds. No draft approval is required. Do not publish
nightly builds or upload build artifacts to source control.

## License

By contributing, you agree that your contributions are distributed under the
MIT License in `LICENSE`. Preserve the CCFDDL copyright notice and do not add
third-party code without confirming that its license is compatible.
