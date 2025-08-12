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
# Start the environment (container will be named devenv-<name>)
devenv start <name>
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
- Packages are installed via `apt-get` when available; other base images are left untouched.
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
- Builds an image that installs `openssh-server` when `apt-get` is available.
- Starts the container with port `22` exposed on host `:2222` (override via `ssh_port`).
- Adds the public key to the container user’s `~/.ssh/authorized_keys` (user order: `zed_remote.ssh_user`, `user_name`, else `root`).

Connect from Zed:
- Target: `ssh://<user>@localhost:2222/workspace` (replace port/user as configured)
- Identity file: `./.devenv/zed_ed25519`

Quick test via CLI:

```sh
ssh -i ./.devenv/zed_ed25519 -p 2222 <user>@localhost 'echo ok'
```

Notes:
- For non-Debian base images (e.g. Alpine), `openssh-server` is not auto-installed; use a Debian/Ubuntu base or add SSH server manually.
- If `ssh-keygen` is unavailable on your host, create keys yourself in `./.devenv/`.

## Commands
- `devenv init [<path>]`: Create Dockerfile/config for a project and register it.
- `devenv list`: List running dev environments (containers named `devenv-*`).
- `devenv start <name> [--open[=CMD]] [--attach] [--rebuild] [--no-build]`: Build/run the environment container, mount project at `/workspace`. If `--open` is provided, opens the project directory in an IDE (defaults to `zed`; override with a custom CLI path, e.g. `--open code` or `--open /path/to/editor`). `--attach` drops you into an interactive shell in the container after it starts. `--rebuild` regenerates the Dockerfile from `devenv.toml` before building. `--no-build` skips the image build step if present.
- `devenv attach <name>`: Open an interactive shell inside the running container.
- `devenv stop <name>`: Stop the environment container.
- `devenv restart <name> [--open[=CMD]] [--attach] [--rebuild] [--no-build]`: Stop if running, then start. Same flags as `start`. If not running, prints an info message and starts anyway.
- `devenv build <name> [--rebuild] [--pull]`: Generate Dockerfile from `devenv.toml` when `--rebuild` is set (or missing Dockerfile) and build the image (optionally `--pull` latest base layers).
- `devenv remove <name>`: Remove the environment container and unregister it.

---

Questions or ideas? PRs and issues welcome!
