mod detect;
mod docker;
mod registry;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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
    Start { name: String },
    /// Stop the named environment
    Stop { name: String },
    /// Remove the environment container and unregister it
    Remove { name: String },
    /// Attach an interactive shell to the environment
    Attach { name: String },
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
}

// Default derived above

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { path } => cmd_init(path),
        Commands::List => cmd_list(),
        Commands::Start { name } => cmd_start(&name),
        Commands::Stop { name } => cmd_stop(&name),
        Commands::Remove { name } => cmd_remove(&name),
        Commands::Attach { name } => cmd_attach(&name),
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

fn cmd_start(name: &str) -> Result<()> {
    let path = registry::lookup_env(name)?;
    let cfg: DevEnvConfig = {
        let s = fs::read_to_string(path.join("devenv.toml"))?;
        toml::from_str(&s)?
    };
    let image_tag = format!("devenv-{}:latest", cfg.devenv.name);
    // Ensure image is built
    docker::docker_build(&path, &image_tag)?;

    let container_name = format!("devenv-{}", cfg.devenv.name);
    let running = docker::is_container_running(&container_name)?;
    if running {
        println!("Environment '{}' is already running.", cfg.devenv.name);
        return Ok(());
    }

    if docker::container_exists(&container_name)? {
        docker::docker_start(&container_name)?;
    } else {
        let ssh_mount = cfg.devenv.ssh_private_key.as_deref();
        docker::docker_run_detached(&container_name, &image_tag, &path, ssh_mount)?;
    }

    // Run provisioning commands if any
    if !cfg.devenv.commands.is_empty() {
        println!("Running provisioning commands...");
        for cmd in &cfg.devenv.commands {
            println!("$ {cmd}");
            docker::docker_exec_shell(&container_name, cmd)?;
        }
    }

    println!("Environment '{}' started.", cfg.devenv.name);
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

fn generate_dockerfile(dev: &DevEnv) -> String {
    let mut lines = vec![];
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
    lines.push("RUN mkdir -p /root/.ssh && chmod 700 /root/.ssh".into());
    lines.push("WORKDIR /workspace".into());
    lines.push("CMD [\"/bin/sh\", \"-lc\", \"tail -f /dev/null\"]".into());
    lines.join("\n")
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
