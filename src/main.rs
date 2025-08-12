mod detect;
mod docker;
mod registry;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "devenv", version, about = "Simple dev environment manager", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize a dev environment in the given project directory
    Init { path: Option<PathBuf> },
    /// List running dev environments
    List,
    /// Start the named environment
    Start(StartArgs),
    /// Stop the named environment
    Stop { name: String },
    /// Remove the environment container and unregister it
    Remove { name: String },
    /// Attach an interactive shell to the environment
    Attach { name: String },
    /// Restart the environment: stop if running, then start (accepts same flags as start)
    Restart(StartArgs),
    /// Build the environment image without starting a container
    Build(BuildArgs),
}

#[derive(Args, Debug)]
struct StartArgs {
    /// Environment name
    name: String,
    /// Open the project in an IDE after start. Optional command, defaults to 'zed'.
    #[arg(long, value_name = "CMD", num_args = 0..=1, default_missing_value = "zed")]
    open: Option<String>,
    /// Attach an interactive shell after starting the environment
    #[arg(long)]
    attach: bool,
    /// Rebuild the Dockerfile from devenv.toml before building
    #[arg(long)]
    rebuild: bool,
    /// Skip building the image if present
    #[arg(long)]
    no_build: bool,
}

#[derive(Args, Debug)]
struct BuildArgs {
    /// Environment name
    name: String,
    /// Rebuild the Dockerfile from devenv.toml before building
    #[arg(long)]
    rebuild: bool,
    /// Always pull newer base layers
    #[arg(long)]
    pull: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DevEnvConfig {
    #[serde(default)]
    devenv: DevEnv,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DevEnv {
    /// Unique environment name (defaults to directory name)
    #[serde(default)]
    name: String,
    /// Base Docker image to use (auto-detected if empty)
    #[serde(default)]
    image: String,
    /// Path to SSH private key to mount into the container (optional)
    #[serde(default)]
    ssh_private_key: Option<String>,
    /// Extra OS packages to install (apt-based)
    #[serde(default)]
    packages: Vec<String>,
    /// Commands to run after container start (provisioning)
    #[serde(default)]
    commands: Vec<String>,
    /// Optional Zed remote configuration
    #[serde(default)]
    zed_remote: Option<ZedRemote>,
    /// Optional path to a public key to add to authorized_keys inside the container
    #[serde(default)]
    ssh_public_key: Option<String>,
    /// Optional non-root user configuration for container login/ownership
    #[serde(default)]
    user_name: Option<String>,
    #[serde(default)]
    user_uid: Option<u32>,
    #[serde(default)]
    user_gid: Option<u32>,
    /// Run provisioning commands as non-root user if available
    #[serde(default)]
    provision_as_non_root: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZedRemote {
    #[serde(default)]
    enabled: bool,
    /// SSH port published on the host; defaults to 2222
    ssh_port: Option<u16>,
    /// SSH username (container user); defaults to root
    ssh_user: Option<String>,
}

// Default derived above

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { path } => cmd_init(path),
        Commands::List => cmd_list(),
        Commands::Start(args) => cmd_start(
            &args.name,
            args.open.as_deref(),
            args.attach,
            args.rebuild,
            args.no_build,
        ),
        Commands::Stop { name } => cmd_stop(&name),
        Commands::Remove { name } => cmd_remove(&name),
        Commands::Attach { name } => cmd_attach(&name),
        Commands::Restart(args) => cmd_restart(
            &args.name,
            args.open.as_deref(),
            args.attach,
            args.rebuild,
            args.no_build,
        ),
        Commands::Build(args) => cmd_build(&args.name, args.rebuild, args.pull),
    }
}

fn cmd_init(path: Option<PathBuf>) -> Result<()> {
    let project_dir = path.unwrap_or_else(|| std::env::current_dir().unwrap());
    if !project_dir.exists() {
        return Err(anyhow!("Path does not exist: {}", project_dir.display()));
    }
    let config_path = project_dir.join("devenv.toml");

    let mut cfg: DevEnvConfig = if config_path.exists() {
        let s = fs::read_to_string(&config_path)
            .with_context(|| format!("Reading {}", config_path.display()))?;
        toml::from_str(&s).with_context(|| "Parsing devenv.toml")?
    } else {
        DevEnvConfig::default()
    };

    // Set defaults
    if cfg.devenv.name.trim().is_empty() {
        let name = project_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("devenv")
            .to_string();
        cfg.devenv.name = name;
    }

    if cfg.devenv.image.trim().is_empty() {
        cfg.devenv.image = detect::detect_base_image(&project_dir)
            .unwrap_or_else(|| "debian:bookworm-slim".to_string());
    }

    // Write config if missing
    if !config_path.exists() {
        let toml_str = toml::to_string_pretty(&cfg)?;
        fs::write(&config_path, toml_str)?;
        println!("Created {}", config_path.display());
    } else {
        println!("Using existing {}", config_path.display());
    }

    // Create a simple Dockerfile using the chosen image
    let dockerfile_path = project_dir.join("Dockerfile");
    if !dockerfile_path.exists() {
        let dockerfile = generate_dockerfile(&cfg.devenv);
        fs::write(&dockerfile_path, dockerfile)?;
        println!("Created {}", dockerfile_path.display());
    } else {
        println!("Found existing Dockerfile; leaving it unchanged");
    }

    // Register environment
    registry::register_env(&cfg.devenv.name, &project_dir)?;
    println!(
        "Registered environment '{}' at {}",
        cfg.devenv.name,
        project_dir.display()
    );

    // Optionally build image now
    let image_tag = format!("devenv-{}:latest", cfg.devenv.name);
    println!(
        "Building image '{}' (FROM {})...",
        image_tag, cfg.devenv.image
    );
    docker::docker_build(&project_dir, &image_tag)?;
    println!("Image built: {image_tag}");

    Ok(())
}

fn cmd_list() -> Result<()> {
    let items = docker::docker_ps_devenv()?;
    if items.is_empty() {
        println!("No running dev environments.");
    } else {
        for it in items {
            println!("{}\t{}\t{}", it.name, it.image, it.status);
        }
    }
    Ok(())
}

fn cmd_start(
    name: &str,
    open_cmd: Option<&str>,
    attach: bool,
    rebuild: bool,
    no_build: bool,
) -> Result<()> {
    let path = registry::lookup_env(name)?;
    let cfg: DevEnvConfig = {
        let s = fs::read_to_string(path.join("devenv.toml"))?;
        toml::from_str(&s)?
    };
    let dockerfile_expected = generate_dockerfile(&cfg.devenv);
    let dockerfile_path = path.join("Dockerfile");
    if rebuild {
        fs::write(&dockerfile_path, &dockerfile_expected)?;
        println!("Rebuilt {} from devenv.toml", dockerfile_path.display());
    } else if dockerfile_path.exists() {
        let current = fs::read_to_string(&dockerfile_path).unwrap_or_default();
        if current != dockerfile_expected {
            eprintln!(
                "Warning: Dockerfile is out of sync with devenv.toml. Run 'devenv start {name} --rebuild' to regenerate."
            );
        }
    } else {
        fs::write(&dockerfile_path, &dockerfile_expected)?;
        println!("Created {} from devenv.toml", dockerfile_path.display());
    }
    let image_tag = format!("devenv-{}:latest", cfg.devenv.name);
    // Ensure image is built unless skipped
    if !no_build {
        docker::docker_build(&path, &image_tag)?;
    }

    let container_name = format!("devenv-{}", cfg.devenv.name);
    let running = docker::is_container_running(&container_name)?;
    if running {
        println!("Environment '{}' is already running.", cfg.devenv.name);
        return Ok(());
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

    if docker::container_exists(&container_name)? {
        docker::docker_start(&container_name)?;
    } else {
        docker::docker_run_detached(&container_name, &image_tag, &path, ssh_port)?;
    }

    // Run provisioning commands if any
    if !cfg.devenv.commands.is_empty() {
        println!("Running provisioning commands...");
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
            println!("$ {cmd}");
            if cfg.devenv.provision_as_non_root {
                if let Some(user) = non_root_user.as_deref() {
                    docker::docker_exec_shell_as(&container_name, user, cmd)?;
                } else {
                    docker::docker_exec_shell(&container_name, cmd)?;
                }
            } else {
                docker::docker_exec_shell(&container_name, cmd)?;
            }
        }
    }

    // If Zed remote is enabled, try to start sshd inside the container
    if let Some(z) = &cfg.devenv.zed_remote
        && z.enabled
    {
        let start_sshd = "mkdir -p /run/sshd && (service ssh start || (which /usr/sbin/sshd && /usr/sbin/sshd) || (which sshd && sshd) || true)";
        let _ = docker::docker_exec_shell(&container_name, start_sshd);
    }

    // Ensure project-managed keys exist and add to authorized_keys; update .gitignore if present
    update_project_gitignore(&path)?;
    let pubkey_path = if let Some(p) = &cfg.devenv.ssh_public_key {
        Some(PathBuf::from(p))
    } else {
        ensure_project_ssh_keys(&path, &cfg.devenv.name)?
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
        let _ = docker::docker_exec_shell(&container_name, &script);
    }

    println!("Environment '{}' started.", cfg.devenv.name);
    if let Some(cmd) = open_cmd {
        println!("Opening project in '{cmd}'...");
        let target = path.to_string_lossy().to_string();
        let _ = Command::new(cmd).arg(&target).spawn();
    }
    if attach {
        return docker::docker_exec_interactive_shell(&container_name);
    }
    Ok(())
}

fn cmd_stop(name: &str) -> Result<()> {
    let container_name = format!("devenv-{name}");
    if !docker::container_exists(&container_name)? {
        println!("Environment '{name}' is not created.");
        return Ok(());
    }
    if docker::is_container_running(&container_name)? {
        docker::docker_stop(&container_name)?;
        println!("Environment '{name}' stopped.");
    } else {
        println!("Environment '{name}' is not running.");
    }
    Ok(())
}

fn cmd_attach(name: &str) -> Result<()> {
    let container_name = format!("devenv-{name}");
    if !docker::container_exists(&container_name)? {
        anyhow::bail!("Environment '{}' does not exist.", name);
    }
    if !docker::is_container_running(&container_name)? {
        anyhow::bail!(
            "Environment '{}' is not running. Use 'devenv start {}' first.",
            name,
            name
        );
    }
    println!("Attaching to '{container_name}'... (exit to detach)");
    docker::docker_exec_interactive_shell(&container_name)
}

fn cmd_remove(name: &str) -> Result<()> {
    let container_name = format!("devenv-{name}");
    if docker::container_exists(&container_name)? {
        if docker::is_container_running(&container_name)? {
            docker::docker_stop(&container_name)?;
            println!("Stopped '{container_name}'");
        }
        docker::docker_remove_container(&container_name, false)?;
        println!("Removed container '{container_name}'");
    } else {
        println!("No container named '{container_name}' found.");
    }

    match registry::unregister_env(name) {
        Ok(true) => println!("Unregistered environment '{name}'"),
        Ok(false) => println!("Environment '{name}' not found in registry."),
        Err(e) => return Err(e),
    }
    Ok(())
}

fn cmd_restart(
    name: &str,
    open_cmd: Option<&str>,
    attach: bool,
    rebuild: bool,
    no_build: bool,
) -> Result<()> {
    let container_name = format!("devenv-{name}");
    match (
        docker::container_exists(&container_name)?,
        docker::is_container_running(&container_name)?,
    ) {
        (true, true) => {
            docker::docker_stop(&container_name)?;
            println!("Environment '{name}' stopped.");
        }
        (true, false) => {
            println!("Environment '{name}' is not running; starting it now.");
        }
        (false, _) => {
            println!("Environment '{name}' not created yet; starting fresh.");
        }
    }
    cmd_start(name, open_cmd, attach, rebuild, no_build)
}

fn cmd_build(name: &str, rebuild: bool, pull: bool) -> Result<()> {
    let path = registry::lookup_env(name)?;
    let cfg: DevEnvConfig = {
        let s = fs::read_to_string(path.join("devenv.toml"))?;
        toml::from_str(&s)?
    };
    let dockerfile_expected = generate_dockerfile(&cfg.devenv);
    let dockerfile_path = path.join("Dockerfile");
    if rebuild || !dockerfile_path.exists() {
        fs::write(&dockerfile_path, &dockerfile_expected)?;
        println!(
            "Dockerfile written from devenv.toml at {}",
            dockerfile_path.display()
        );
    } else {
        let current = fs::read_to_string(&dockerfile_path).unwrap_or_default();
        if current != dockerfile_expected {
            eprintln!("Warning: Dockerfile differs from generated; consider --rebuild.");
        }
    }
    let image_tag = format!("devenv-{}:latest", cfg.devenv.name);
    println!(
        "Building image '{}' (FROM {})...",
        image_tag, cfg.devenv.image
    );
    docker::docker_build_with_opts(&path, &image_tag, pull)?;
    println!("Image built: {image_tag}");
    Ok(())
}

fn generate_dockerfile(dev: &DevEnv) -> String {
    let mut lines = vec![];
    lines.push("# Generated by devenv. Do not edit manually.".into());
    lines.push("# Edit devenv.toml and use 'devenv start --rebuild' instead.".into());
    lines.push(format!("FROM {}", dev.image));
    lines.push("\n# Common utilities".into());
    lines.push("RUN mkdir -p /workspace && \\".into());
    lines.push("    (command -v apt-get >/dev/null 2>&1 && apt-get update && apt-get install -y curl ca-certificates git sudo) || true".into());
    if !dev.packages.is_empty() {
        let pkgs = dev.packages.join(" ");
        lines.push(format!(
            "RUN (command -v apt-get >/dev/null 2>&1 && apt-get update && apt-get install -y {pkgs}) || true"
        ));
    }

    // Optional: create a non-root user inside the image
    if let Some(user) = dev
        .user_name
        .as_deref()
        .or_else(|| dev.zed_remote.as_ref().and_then(|z| z.ssh_user.as_deref()))
        && user != "root"
    {
        let uid = dev.user_uid.unwrap_or(1000);
        let gid = dev.user_gid.unwrap_or(uid);
        lines.push(format!(
            "RUN (getent group {gid} || groupadd -g {gid} {user}) || true"
        ));
        lines.push(format!("RUN (id -u {user} >/dev/null 2>&1 || useradd -m -u {uid} -g {gid} -s /bin/bash {user}) || true"));
        lines.push(format!(
            "RUN mkdir -p /home/{user}/.ssh && chown -R {user}:{user} /home/{user}"
        ));
    }
    lines.push("RUN mkdir -p /root/.ssh && chmod 700 /root/.ssh".into());
    lines.push("WORKDIR /workspace".into());
    lines.push("CMD [\"/bin/sh\", \"-lc\", \"tail -f /dev/null\"]".into());
    lines.join("\n")
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
        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-C"])
            .arg(&label)
            .args(["-f"])
            .arg(&priv_key)
            .status();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dockerfile_includes_from_and_workdir() {
        let dev = DevEnv {
            image: "debian:bookworm-slim".into(),
            ..Default::default()
        };
        let df = generate_dockerfile(&dev);
        assert!(df.contains("FROM debian:bookworm-slim"));
        assert!(df.contains("WORKDIR /workspace"));
    }

    #[test]
    fn dockerfile_includes_packages_when_present() {
        let dev = DevEnv {
            image: "debian:bookworm-slim".into(),
            packages: vec!["make".into(), "git".into()],
            ..Default::default()
        };
        let df = generate_dockerfile(&dev);
        assert!(df.contains("apt-get install -y make git"));
    }
}
