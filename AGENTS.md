# Repository Guidelines

## Project Structure & Module Organization
- `crates/app`: Binary entry point and wiring (config loading, runtime bootstrap).
- `crates/core`: Functional core (events, state, reducers, deciders) with most logic and tests.
- `crates/drivers`: IO drivers for NapCat WS and Qzone sending.
- `crates/infra`: Persistence primitives (journal, snapshot, blob).
- `docs/`: Design, runbooks, and configuration references.
- `res/`: Rendering resources and assets used by the renderer.
- `data/`: Runtime data (snapshots, journals). Keep out of version control.

## Build, Test, and Development Commands
- `cargo run -p OQQWall_RUST -- oobe`: Generate a starter `config.json`.
- `cargo run -p OQQWall_RUST`: Run the service locally (reads `./config.json` by default).
- `cargo test`: Run all tests across the workspace.
- `cargo test -p core`: Focus on functional core correctness.
- `cargo fmt --check`: Enforce formatting.
- `cargo clippy -D warnings`: Run linting as errors.
- `cargo check`: Run after each change to catch compile errors early.

## Coding Style & Naming Conventions
- Rustfmt is the source of truth for formatting; use 4-space indentation.
- Follow Rust naming: `snake_case` for modules/functions, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Keep pure logic in `crates/core` and IO in `crates/drivers` to preserve the Functional Core / Imperative Shell split.

## Testing Guidelines
- Core tests live in `crates/core/tests` (for example, `reduce_replay.rs`, `decide_tick.rs`).
- Prioritize reducer replay, decider tick behavior, and command parsing stability.
- Run `cargo test -p core` before touching IO-heavy paths.

## Commit & Pull Request Guidelines
- Commit history uses short, lowercase summaries (for example, `add cqface`); keep subjects concise and action-oriented.
- PRs should include a clear description, the tests you ran, and note any config or behavior changes (especially `config.json`/`devconfig.json`).

## Configuration & Runtime Notes
- Use `OQQWALL_NAPCAT_TOKEN` to avoid committing access tokens.
- `devconfig.json` is read only in debug builds; release ignores it.
- Persistent data and snapshots belong under `data/`; do not commit generated files in `target/`.
