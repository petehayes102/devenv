# devenv

[![CI](https://github.com/petehayes102/devenv/actions/workflows/ci.yml/badge.svg)](https://github.com/petehayes102/devenv/actions/workflows/ci.yml)

A simple CLI to turn any project into a reproducible dev environment using Docker. It auto-detects common languages to choose a base image, generates a Dockerfile and a `devenv.toml`, and lets you start/stop environments by name.

## Install

```sh
cargo install devenv
```

Requirements: Docker must be available on your system.

## Quick Start

```sh
cd /path/to/your/project
# Initialize (creates Dockerfile and devenv.toml and builds the image)
devenv init
# Start the environment
# - With a name: uses the registered project
# - Without a name: reads ./devenv.toml in the current directory
# - Add --verbose to show docker output
devenv start [<name>] [--verbose]
# List running dev environments
devenv list
# Attach a shell to a running environment
devenv attach <name>
# Stop the environment
devenv stop <name>
```

## Configuration (devenv.toml)
A minimal config is created on `init`. You can customize it:

```toml
[devenv]
name = "my-project"                 # defaults to directory name
image = "rust:latest"               # auto-detected if empty
packages = ["build-essential"]       # optional apt packages
commands = ["cargo --version"]       # optional provisioning commands
user_name = "dev"                    # optional non-root user
user_uid = 1000
user_gid = 1000
provision_as_non_root = true          # run provisioning commands as non-root user (if available)
```

- The Dockerfile is generated from the selected `image` and includes basic utilities.
- Packages are installed via `apt` when available; other base images are left untouched.
- Dockerfile management: devenv owns the Dockerfile. If it’s out of sync with `devenv.toml`, `devenv start` will warn; use `--rebuild` to regenerate it.

### Working in the container

Use `--attach` to drop into a shell after starting:

```
devenv start <name> --attach
```

The `--open` flag just launches your local editor on the project directory; it does not configure remote editing.

## Zed Remote
Enable SSH-based remote editing with Zed, with keys managed inside your project under `./.devenv/`.

Minimal `devenv.toml` snippet:

```toml
[devenv]
name = "my-project"
image = "debian:bookworm-slim"

[devenv.zed_remote]
enabled = true           # turn on SSH + port publish
ssh_port = 2222          # optional (default 2222)
ssh_user = "root"        # optional (or set [devenv.user_name])
```

Behavior when enabled:
- Generates an `ed25519` keypair at `./.devenv/zed_ed25519(.pub)` if missing.
- Appends `/.devenv` to `.gitignore` if present.
- Starts the container with port `22` exposed on host `:2222` (override via `ssh_port`).
- Adds the public key to the container user’s `~/.ssh/authorized_keys` (user order: `zed_remote.ssh_user`, `user_name`, else `root`).

Connect from Zed:
- Target: `ssh://<user>@localhost:2222/workspace` (replace port/user as configured)
- Identity file: `./.devenv/zed_ed25519`

## Commands
- `devenv init [<path>]`: Create Dockerfile/config for a project and register it.
- `devenv list`: List running dev environments (containers named `devenv-*`).
- `devenv start [<name>] [--open[=CMD]] [--attach] [--rebuild] [--no-build] [--verbose]`: Build/run the environment container. When `<name>` is omitted, devenv looks for `./devenv.toml` in the current directory and derives the name/config from it. Mounts the project at `/workspace`. If `--open` is provided, opens the project directory in an IDE (defaults to `zed`; override with a custom CLI path, e.g. `--open code` or `--open /path/to/editor`). `--attach` drops you into an interactive shell in the container after it starts. `--rebuild` regenerates the Dockerfile from `devenv.toml` before building. `--no-build` skips the image build step if present. `--verbose` prints subprocess output.
- `devenv attach [<name>] [--verbose]`: Open an interactive shell inside the running container. When `<name>` is omitted, devenv uses `./devenv.toml` in the current directory to determine the environment.
- `devenv stop [<name>] [--verbose]`: Stop the environment container. When `<name>` is omitted, devenv uses `./devenv.toml` in the current directory.
- `devenv restart [<name>] [--open[=CMD]] [--attach] [--rebuild] [--no-build] [--verbose]`: Stop if running, then start. Same flags and name behavior as `start`. If not running, prints an info message and starts anyway.
- `devenv build [<name>] [--rebuild] [--pull] [--verbose]`: Generate Dockerfile from `devenv.toml` when `--rebuild` is set (or when Dockerfile is missing) and build the image. When `<name>` is omitted, devenv reads `./devenv.toml` in the current directory. `--verbose` prints subprocess output.
- `devenv remove [<name>] [--verbose]`: Remove the environment container and unregister it. When `<name>` is omitted, devenv uses `./devenv.toml` in the current directory.

### Logging
- `--verbose`: Prints subprocess output (e.g. docker, ssh-keygen). Without it, devenv logs the high-level commands it runs and suppresses child stdout/stderr.
- `RUST_LOG=info|warn|debug`: Controls devenv's own log level (e.g., `RUST_LOG=debug devenv start`).

---

Questions or ideas? PRs and issues welcome!
