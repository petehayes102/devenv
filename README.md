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
ssh_private_key = "~/.ssh/id_rsa"   # optional, mounted read-only
packages = ["build-essential"]       # optional apt packages
commands = ["cargo --version"]       # optional provisioning commands
```

- The Dockerfile is generated from the selected `image` and includes basic utilities.
- Packages are installed via `apt-get` when available; other base images are left untouched.

## Commands
- `devenv init [<path>]`: Create Dockerfile/config for a project and register it.
- `devenv list`: List running dev environments (containers named `devenv-*`).
- `devenv start <name> [--open[=CMD]] [--attach]`: Build/run the environment container, mount project at `/workspace`. If `--open` is provided, opens the project directory in an IDE (defaults to `zed`; override with a custom CLI path, e.g. `--open code` or `--open /path/to/editor`). `--attach` drops you into an interactive shell in the container after it starts.
- `devenv attach <name>`: Open an interactive shell inside the running container.
- `devenv stop <name>`: Stop the environment container.
- `devenv remove <name>`: Remove the environment container and unregister it.

---

Questions or ideas? PRs and issues welcome!
