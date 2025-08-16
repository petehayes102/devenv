use std::{fs, path::PathBuf, process::Command};

use anyhow::{Result, anyhow, bail};
use clap::Parser;
use tracing::{Level, info, warn};
use tracing_subscriber::EnvFilter;

use crate::{
    cli::{Cli, Commands},
    config::Config,
    docker::{
        DockerClient,
        file::{Dockerfile, OsFamily},
    },
};

mod cli;
mod config;
mod detect;
mod docker;
mod registry;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_level(false)
        .with_target(false)
        .without_time()
        .with_max_level(if cli.verbose {
            Level::TRACE
        } else {
            Level::WARN
        })
        .try_init()
        .map_err(|e| anyhow!(e))?;

    return match cli.command {
        Commands::Init { path } => cmd_init(path).await,
        Commands::List => cmd_list().await,
        Commands::Start(args) => {
            cmd_start(
                args.name.as_deref(),
                args.open.as_deref(),
                args.attach,
                args.rebuild,
                args.no_build,
            )
            .await
        }
        Commands::Stop { name } => cmd_stop(name.as_deref()).await,
        Commands::Remove { name } => cmd_remove(name.as_deref()).await,
        Commands::Attach { name } => cmd_attach(name.as_deref()).await,
        Commands::Restart(args) => {
            cmd_restart(
                args.name.as_deref(),
                args.open.as_deref(),
                args.attach,
                args.rebuild,
                args.no_build,
            )
            .await
        }
        Commands::Build(args) => cmd_build(args.name.as_deref(), args.rebuild, args.pull).await,
    };
}

async fn cmd_init(path: Option<PathBuf>) -> Result<()> {
    let project_dir = path.unwrap_or_else(|| std::env::current_dir().unwrap());
    if !project_dir.exists() {
        bail!("Path does not exist: {}", project_dir.display());
    }

    // Create devenv.toml
    let cfg = if Config::exists(&project_dir) {
        let cfg = Config::open(&project_dir)?;
        info!("Using existing {}", cfg.path.display());
        cfg
    } else {
        let cfg = Config::create(&project_dir)?;
        info!("Created {}", cfg.path.display());
        cfg
    };

    // Create Dockerfile
    if Dockerfile::exists(&project_dir) {
        info!("Found existing Dockerfile; leaving it unchanged");
    } else {
        let dockerfile =
            Dockerfile::create(&cfg.devenv.image, &cfg.devenv.packages, OsFamily::Debian)?;
        dockerfile.write(&project_dir)?;
        info!("Created Dockerfile in {}", project_dir.display());
    }

    // Register environment in global registry
    registry::register_env(&cfg.devenv.name, &project_dir)?;
    info!(
        "Registered environment '{}' at {}",
        cfg.devenv.name,
        project_dir.display()
    );

    // Build image now
    let image_tag = format!("devenv-{}:latest", cfg.devenv.name);
    info!(
        "Building image '{}' (FROM {})...",
        image_tag, cfg.devenv.image
    );
    let docker = DockerClient::new()?;
    docker
        .build_with_opts(&project_dir, &image_tag, false, false)
        .await?;
    info!("Image built: {image_tag}");

    Ok(())
}

async fn cmd_list() -> Result<()> {
    let docker = DockerClient::new()?;
    let items = docker.ps().await?;
    if items.is_empty() {
        info!("No running dev environments");
    } else {
        for it in items {
            info!("{}\t{}\t{}", it.name, it.image, it.status);
        }
    }
    Ok(())
}

async fn cmd_start(
    name: Option<&str>,
    open_cmd: Option<&str>,
    attach: bool,
    rebuild: bool,
    no_build: bool,
) -> Result<()> {
    let project_dir = resolve_env(name)?;
    let cfg = Config::open(&project_dir)?;

    // Check if the environment has already started
    let container_name = format!("devenv-{}", cfg.devenv.name);
    let docker = DockerClient::new()?;
    let running = docker.is_container_running(&container_name).await?;
    if running {
        info!("Environment '{}' is already running.", cfg.devenv.name);
        return Ok(());
    }

    // Create/rebuild Dockerfile as necessary
    let expected = Dockerfile::create(&cfg.devenv.image, &cfg.devenv.packages, OsFamily::Debian)?;
    if Dockerfile::exists(&project_dir) {
        let current = Dockerfile::open(&project_dir)?;
        if current != expected {
            warn!(
                "Warning: Dockerfile is out of sync with devenv.toml. Use the `--rebuild` flag to regenerate."
            );
        }
    } else {
        expected.write(&project_dir)?;
        info!("Rebuilt {} from devenv.toml", project_dir.display());
    }

    // Build image unless user asks us not to
    let image_tag = format!("devenv-{}:latest", cfg.devenv.name);
    if !no_build {
        docker
            .build_with_opts(&project_dir, &image_tag, false, rebuild)
            .await?;
    }

    // Determine SSH port if Zed remote is enabled
    let ssh_port: Option<u16> = cfg
        .devenv
        .zed_remote
        .as_ref()
        .filter(|z| z.enabled)
        .and_then(|z| z.ssh_port)
        .or_else(|| {
            cfg.devenv
                .zed_remote
                .as_ref()
                .and_then(|z| if z.enabled { Some(2222) } else { None })
        });

    if docker.container_exists(&container_name).await? {
        docker.start(&container_name).await?;
    } else {
        docker
            .run_detached(&container_name, &image_tag, &project_dir, ssh_port)
            .await?;
    }

    // Run provisioning commands if any
    if !cfg.devenv.commands.is_empty() {
        info!("Running provisioning commands...");
        // Choose user to run provisioning
        let non_root_user = cfg
            .devenv
            .user_name
            .clone()
            .or_else(|| {
                cfg.devenv
                    .zed_remote
                    .as_ref()
                    .and_then(|z| z.ssh_user.clone())
            })
            .filter(|u| u != "root");
        for cmd in &cfg.devenv.commands {
            info!("$ {cmd}");
            if cfg.devenv.provision_as_non_root {
                if let Some(user) = non_root_user.as_deref() {
                    docker.exec_shell_as(&container_name, user, cmd).await?;
                } else {
                    docker.exec_shell(&container_name, cmd).await?;
                }
            } else {
                docker.exec_shell(&container_name, cmd).await?;
            }
        }
    }

    // If Zed remote is enabled, try to start sshd inside the container
    if let Some(z) = &cfg.devenv.zed_remote
        && z.enabled
    {
        let start_sshd = "mkdir -p /run/sshd && (service ssh start || (which /usr/sbin/sshd && /usr/sbin/sshd) || (which sshd && sshd) || true)";
        let _ = docker.exec_shell(&container_name, start_sshd).await;
    }

    // Ensure project-managed keys exist and add to authorized_keys; update .gitignore if present
    update_project_gitignore(&project_dir)?;
    let pubkey_path = if let Some(p) = &cfg.devenv.ssh_public_key {
        Some(PathBuf::from(p))
    } else {
        ensure_project_ssh_keys(&project_dir, &cfg.devenv.name)?
    };
    if let Some(pubkey_path) = pubkey_path
        && let Ok(key) = fs::read_to_string(&pubkey_path)
    {
        let user = cfg
            .devenv
            .zed_remote
            .as_ref()
            .and_then(|z| z.ssh_user.clone())
            .or_else(|| cfg.devenv.user_name.clone())
            .unwrap_or_else(|| "root".to_string());
        let home = if user == "root" {
            "/root".to_string()
        } else {
            format!("/home/{user}")
        };
        let script = format!(
            "install -d -m 700 {home}/.ssh && printf '%s\\n' '{key}' > {home}/.ssh/authorized_keys && chown -R {user}:{user} {home}/.ssh && chmod 600 {home}/.ssh/authorized_keys",
            home = home,
            user = user,
            key = key.trim().replace("'", "'\\''"),
        );
        let _ = docker.exec_shell(&container_name, &script).await;
    }

    info!("Environment '{}' started.", cfg.devenv.name);
    if let Some(cmd) = open_cmd {
        info!("Opening project in '{cmd}'...");
        let target = project_dir.to_string_lossy().to_string();
        let _ = Command::new(cmd).arg(&target).spawn();
    }
    if attach {
        return docker.exec_interactive_shell(&container_name).await;
    }
    Ok(())
}

async fn cmd_stop(name: Option<&str>) -> Result<()> {
    let effective_name = if let Some(n) = name {
        n.to_string()
    } else {
        let path = resolve_env(None)?;
        let cfg = Config::open(&path)?;
        cfg.devenv.name
    };
    let container_name = format!("devenv-{}", effective_name);
    let docker = DockerClient::new()?;
    if !docker.container_exists(&container_name).await? {
        info!("Environment '{}' is not created.", effective_name);
        return Ok(());
    }
    if docker.is_container_running(&container_name).await? {
        docker.stop(&container_name).await?;
        info!("Environment '{}' stopped.", effective_name);
    } else {
        info!("Environment '{}' is not running.", effective_name);
    }
    Ok(())
}

async fn cmd_attach(name: Option<&str>) -> Result<()> {
    let effective_name = if let Some(n) = name {
        n.to_string()
    } else {
        let path = resolve_env(None)?;
        let cfg = Config::open(&path)?;
        cfg.devenv.name
    };
    let container_name = format!("devenv-{}", effective_name);
    let docker = DockerClient::new()?;
    if !docker.container_exists(&container_name).await? {
        anyhow::bail!("Environment '{}' does not exist.", effective_name);
    }
    if !docker.is_container_running(&container_name).await? {
        let hint = if let Some(n) = name {
            format!("devenv start {n}")
        } else {
            "devenv start".to_string()
        };
        anyhow::bail!(
            "Environment '{}' is not running. Use '{}' first.",
            effective_name,
            hint
        );
    }
    info!("Attaching to '{container_name}'... (exit to detach)");
    docker.exec_interactive_shell(&container_name).await
}

async fn cmd_remove(name: Option<&str>) -> Result<()> {
    let effective_name = if let Some(n) = name {
        n.to_string()
    } else {
        let path = resolve_env(None)?;
        let cfg = Config::open(&path)?;
        cfg.devenv.name
    };
    let container_name = format!("devenv-{}", effective_name);
    let docker = DockerClient::new()?;
    if docker.container_exists(&container_name).await? {
        if docker.is_container_running(&container_name).await? {
            docker.stop(&container_name).await?;
            info!("Stopped '{container_name}'");
        }
        docker.remove_container(&container_name, false).await?;
        info!("Removed container '{container_name}'");
    } else {
        info!("No container named '{container_name}' found.");
    }

    match registry::unregister_env(&effective_name) {
        Ok(true) => info!("Unregistered environment '{}'", effective_name),
        Ok(false) => info!("Environment '{}' not found in registry.", effective_name),
        Err(e) => return Err(e),
    }
    Ok(())
}

async fn cmd_restart(
    name: Option<&str>,
    open_cmd: Option<&str>,
    attach: bool,
    rebuild: bool,
    no_build: bool,
) -> Result<()> {
    // Resolve container name from registry or current directory config
    let effective_name = if let Some(n) = name {
        n.to_string()
    } else {
        let path = resolve_env(None)?;
        let cfg = Config::open(&path)?;
        cfg.devenv.name
    };
    let container_name = format!("devenv-{}", effective_name);
    let docker = DockerClient::new()?;
    match (
        docker.container_exists(&container_name).await?,
        docker.is_container_running(&container_name).await?,
    ) {
        (true, true) => {
            docker.stop(&container_name).await?;
            info!("Environment '{}' stopped.", effective_name);
        }
        (true, false) => {
            info!(
                "Environment '{}' is not running; starting it now.",
                effective_name
            );
        }
        (false, _) => {
            info!(
                "Environment '{}' not created yet; starting fresh.",
                effective_name
            );
        }
    }
    cmd_start(name, open_cmd, attach, rebuild, no_build).await
}

async fn cmd_build(name: Option<&str>, rebuild: bool, pull: bool) -> Result<()> {
    let path = resolve_env(name)?;
    let cfg = Config::open(&path)?;

    let expected = Dockerfile::create(&cfg.devenv.image, &cfg.devenv.packages, OsFamily::Debian)?;
    if rebuild || !Dockerfile::exists(&path) {
        expected.write(&path)?;
        info!("Dockerfile written from devenv.toml at {}", path.display());
    } else {
        let current = Dockerfile::open(&path)?;
        if current != expected {
            warn!("Warning: Dockerfile differs from generated; consider --rebuild.");
        }
    }
    let image_tag = format!("devenv-{}:latest", cfg.devenv.name);
    info!(
        "Building image '{}' (FROM {})...",
        image_tag, cfg.devenv.image
    );
    let docker = DockerClient::new()?;
    docker
        .build_with_opts(&path, &image_tag, pull, false)
        .await?;
    info!("Image built: {image_tag}");
    Ok(())
}

// Resolve environment by:
// 1. User-provided project name via Registry, or
// 2. By looking for `devenv.toml` in CWD
fn resolve_env(name: Option<&str>) -> Result<PathBuf> {
    Ok(match name {
        Some(name) => registry::lookup_env(name)?,
        None => std::env::current_dir()?,
    })
}

// Ensure project-level SSH keys under ./.devenv; returns pubkey path if available
fn ensure_project_ssh_keys(
    project_dir: &std::path::Path,
    env_name: &str,
) -> Result<Option<PathBuf>> {
    let devenv_dir = project_dir.join(".devenv");
    if !devenv_dir.exists() {
        let _ = fs::create_dir_all(&devenv_dir);
    }
    let priv_key = devenv_dir.join("zed_ed25519");
    let pub_key = devenv_dir.join("zed_ed25519.pub");
    if !priv_key.exists() || !pub_key.exists() {
        let label = format!("devenv-{env_name} zed");
        let mut cmd = Command::new("ssh-keygen");
        cmd.args(["-t", "ed25519", "-N", "", "-C"])
            .arg(&label)
            .args(["-f"])
            .arg(&priv_key);
        tracing::info!(
            "$ ssh-keygen -t ed25519 -N '' -C '{}' -f {}",
            label,
            priv_key.display()
        );
        let status = cmd.status();
        if !matches!(status, Ok(s) if s.success()) {
            return Ok(None);
        }
    }
    Ok(Some(pub_key))
}

// If .gitignore exists, ensure it ignores '/.devenv'
fn update_project_gitignore(project_dir: &std::path::Path) -> Result<()> {
    let gi = project_dir.join(".gitignore");
    if gi.exists() {
        let mut content = fs::read_to_string(&gi).unwrap_or_default();
        let line = "/.devenv";
        if !content.lines().any(|l| l.trim() == line) {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(line);
            content.push('\n');
            fs::write(gi, content)?;
        }
    }
    Ok(())
}
