# Repository Guidelines

This repository contains a Rust CLI named `devenv` that provisions reproducible development environments using Docker (via the `bollard` crate). Use this guide to develop, test, and contribute changes efficiently.

## Project Structure & Module Organization
- `src/main.rs`: CLI entrypoint (async with Tokio).
- `src/cli.rs`: Clap argument parsing and subcommands.
- `src/config.rs`: `devenv.toml` schema and I/O.
- `src/detect.rs`: Language/image detection.
- `src/docker/`: Docker integration
  - `mod.rs`: Container lifecycle, build, exec/attach.
  - `file.rs`: Dockerfile generation.
- `src/registry.rs`: Project registry helpers.
- `README.md`: Usage overview.

## Build, Test, and Development Commands
- Build: `cargo build` — compiles the `devenv` binary.
- Run: `cargo run -- <args>` (e.g., `cargo run -- start --attach`).
- Format: `cargo fmt --all` — formats the codebase.
- Lint: `cargo clippy --all-targets --all-features -- -D warnings` — lints with warnings denied.
- Test: `cargo test --all` — executes the test suite.

## Coding Style & Naming Conventions
- Rust edition: 2024. Use idiomatic Rust and keep functions small and focused.
- Formatting: rustfmt defaults (run before committing).
- Linting: Clippy must pass with `-D warnings`.
- Naming: snake_case for functions/modules, CamelCase for types, SCREAMING_SNAKE_CASE for consts.
- Avoid unnecessary abstractions; keep changes minimal and targeted.

## Testing Guidelines
- Add unit tests near changed modules when feasible.
- Prefer deterministic tests; avoid network/Docker calls in unit tests. Use abstractions or small fixtures instead.
- Name tests clearly (e.g., `mod tests { #[test] fn builds_expected_dockerfile() { ... } }`).

## Commit & Pull Request Guidelines
- Commits: Use clear, imperative messages (e.g., "refactor: switch to bollard for build"). Group related edits.
- PRs: Include a concise description, rationale, and screenshots/terminal output if user-facing behavior changes.
- Check list before opening PR:
  - `cargo fmt --all`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all`

## Security & Configuration Tips
- Requires a reachable Docker daemon. `bollard` uses local defaults (`DOCKER_HOST` respected).
- Avoid committing secrets. Project-managed SSH keys live under `./.devenv/` and should be gitignored.

## Agent-Specific Instructions
- After any Rust changeset, always run: `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test`.
- Keep public APIs stable unless coordinated; prefer targeted refactors with clear migration notes.
