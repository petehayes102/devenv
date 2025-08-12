use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct PsItem {
    pub name: String,
    pub image: String,
    pub status: String,
}

pub fn docker_build(context_dir: &Path, tag: &str) -> Result<()> {
    let status = Command::new("docker")
        .arg("build")
        .arg("-t")
        .arg(tag)
        .arg(context_dir)
        .status()
        .with_context(|| "Failed to spawn docker build")?;
    if !status.success() {
        return Err(anyhow!("docker build failed"));
    }
    Ok(())
}

pub fn docker_build_with_opts(context_dir: &Path, tag: &str, pull: bool) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.arg("build");
    if pull {
        cmd.arg("--pull");
    }
    let status = cmd
        .arg("-t")
        .arg(tag)
        .arg(context_dir)
        .status()
        .with_context(|| "Failed to spawn docker build")?;
    if !status.success() {
        return Err(anyhow!("docker build failed"));
    }
    Ok(())
}

pub fn docker_ps_devenv() -> Result<Vec<PsItem>> {
    let output = Command::new("docker")
        .args([
            "ps",
            "--filter",
            "name=^/devenv-",
            "--format",
            "{{.Names}}\t{{.Image}}\t{{.Status}}",
        ])
        .output()
        .with_context(|| "Failed to run docker ps")?;
    if !output.status.success() {
        return Ok(vec![]);
    }
    let s = String::from_utf8_lossy(&output.stdout);
    Ok(s.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            Some(PsItem {
                name: parts.next()?.to_string(),
                image: parts.next()?.to_string(),
                status: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect())
}

pub fn container_exists(name: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
        .with_context(|| "Failed to run docker ps -a")?;
    if !output.status.success() {
        return Ok(false);
    }
    let s = String::from_utf8_lossy(&output.stdout);
    Ok(s.lines().any(|n| n.trim() == name))
}

pub fn is_container_running(name: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["ps", "--format", "{{.Names}}"])
        .output()
        .with_context(|| "Failed to run docker ps")?;
    if !output.status.success() {
        return Ok(false);
    }
    let s = String::from_utf8_lossy(&output.stdout);
    Ok(s.lines().any(|n| n.trim() == name))
}

pub fn docker_start(name: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["start", name])
        .status()
        .with_context(|| "Failed to run docker start")?;
    if !status.success() {
        return Err(anyhow!("docker start failed"));
    }
    Ok(())
}

pub fn docker_stop(name: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["stop", name])
        .status()
        .with_context(|| "Failed to run docker stop")?;
    if !status.success() {
        return Err(anyhow!("docker stop failed"));
    }
    Ok(())
}

pub fn docker_remove_container(name: &str, force: bool) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.arg("rm");
    if force {
        cmd.arg("-f");
    }
    cmd.arg(name);
    let status = cmd.status().with_context(|| "Failed to run docker rm")?;
    if !status.success() {
        return Err(anyhow!("docker rm failed"));
    }
    Ok(())
}

pub fn docker_run_detached(
    container_name: &str,
    image: &str,
    project_dir: &Path,
    host_ssh_port: Option<u16>,
) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.arg("run")
        .arg("-d")
        .arg("--name")
        .arg(container_name)
        .arg("-v")
        .arg(format!("{}:/workspace", project_dir.display()))
        .arg("-w")
        .arg("/workspace");

    if let Some(port) = host_ssh_port {
        cmd.arg("-p").arg(format!("{port}:22"));
    }

    cmd.arg(image)
        .arg("/bin/sh")
        .arg("-lc")
        .arg("tail -f /dev/null");

    let status = cmd.status().with_context(|| "Failed to run docker run")?;
    if !status.success() {
        return Err(anyhow!("docker run failed"));
    }
    Ok(())
}

pub fn docker_exec_shell(container_name: &str, script: &str) -> Result<()> {
    // Try bash, fallback to sh
    let status = Command::new("docker")
        .args(["exec", container_name, "/bin/bash", "-lc", script])
        .status();
    let ok = matches!(status, Ok(s) if s.success());
    if ok {
        return Ok(());
    }
    let status = Command::new("docker")
        .args(["exec", container_name, "/bin/sh", "-lc", script])
        .status()
        .with_context(|| "Failed to run docker exec")?;
    if !status.success() {
        return Err(anyhow!("docker exec failed"));
    }
    Ok(())
}

pub fn docker_exec_shell_as(container_name: &str, user: &str, script: &str) -> Result<()> {
    // Try bash, fallback to sh
    let status = Command::new("docker")
        .args([
            "exec",
            "-u",
            user,
            container_name,
            "/bin/bash",
            "-lc",
            script,
        ])
        .status();
    let ok = matches!(status, Ok(s) if s.success());
    if ok {
        return Ok(());
    }
    let status = Command::new("docker")
        .args(["exec", "-u", user, container_name, "/bin/sh", "-lc", script])
        .status()
        .with_context(|| "Failed to run docker exec -u")?;
    if !status.success() {
        return Err(anyhow!("docker exec -u failed"));
    }
    Ok(())
}

pub fn docker_exec_interactive_shell(container_name: &str) -> Result<()> {
    // Try bash login shell first
    let status = Command::new("docker")
        .args(["exec", "-it", container_name, "/bin/bash", "-l"])
        .status();
    let ok = matches!(status, Ok(s) if s.success());
    if ok {
        return Ok(());
    }
    // Fallback to sh login shell
    let status = Command::new("docker")
        .args(["exec", "-it", container_name, "/bin/sh", "-l"])
        .status()
        .with_context(|| "Failed to run docker exec -it")?;
    if !status.success() {
        return Err(anyhow!("failed to attach interactive shell"));
    }
    Ok(())
}
