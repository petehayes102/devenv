use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use bollard::query_parameters as qp;
use bollard::{
    Docker,
    exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults},
};
use bytes::Bytes;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures_util::StreamExt;
use http_body_util::{Either, Full};
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use walkdir::WalkDir;

pub mod file;

#[derive(Debug, Clone)]
pub struct PsItem {
    pub name: String,
    pub image: String,
    pub status: String,
}

fn docker_client() -> Result<Docker> {
    Docker::connect_with_local_defaults().map_err(|e| anyhow!(e))
}

pub async fn docker_build_with_opts(
    context_dir: &Path,
    tag: &str,
    pull: bool,
    no_cache: bool,
) -> Result<()> {
    let docker = docker_client()?;
    let tar = create_tar_from_dir(context_dir)?;
    let opts = qp::BuildImageOptionsBuilder::default()
        .dockerfile("Dockerfile")
        .t(tag)
        .pull(if pull { "true" } else { "false" })
        .nocache(no_cache)
        .rm(true)
        .build();
    let body = Either::Left(Full::new(Bytes::from(tar)));
    let mut stream = docker.build_image(opts, None, Some(body));
    while let Some(_msg) = stream.next().await.transpose().unwrap_or(None) {
        // We could log build output here if verbose
    }
    Ok(())
}

pub async fn docker_ps_devenv() -> Result<Vec<PsItem>> {
    let docker = docker_client()?;
    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
    filters.insert("name".into(), vec!["devenv-".into()]);
    let containers = docker
        .list_containers(Some(qp::ListContainersOptions {
            all: false,
            filters: Some(filters),
            ..Default::default()
        }))
        .await?;
    let mut out = Vec::new();
    for c in containers {
        let name = c
            .names
            .as_ref()
            .and_then(|v| v.first())
            .map(|s| s.trim_start_matches('/').to_string())
            .unwrap_or_default();
        let image = c.image.unwrap_or_default();
        let status = c.status.unwrap_or_default();
        out.push(PsItem {
            name,
            image,
            status,
        });
    }
    Ok(out)
}

pub async fn container_exists(name: &str) -> Result<bool> {
    let docker = docker_client()?;
    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
    filters.insert("name".into(), vec![name.to_string()]);
    let containers = docker
        .list_containers(Some(qp::ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        }))
        .await?;
    Ok(!containers.is_empty())
}

pub async fn is_container_running(name: &str) -> Result<bool> {
    let docker = docker_client()?;
    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
    filters.insert("name".into(), vec![name.to_string()]);
    let containers = docker
        .list_containers(Some(qp::ListContainersOptions {
            all: false,
            filters: Some(filters),
            ..Default::default()
        }))
        .await?;
    Ok(!containers.is_empty())
}

pub async fn docker_start(name: &str) -> Result<()> {
    let docker = docker_client()?;
    docker
        .start_container(name, None::<qp::StartContainerOptions>)
        .await?;
    Ok(())
}

pub async fn docker_stop(name: &str) -> Result<()> {
    let docker = docker_client()?;
    docker
        .stop_container(name, None::<qp::StopContainerOptions>)
        .await?;
    Ok(())
}

pub async fn docker_remove_container(name: &str, force: bool) -> Result<()> {
    let docker = docker_client()?;
    docker
        .remove_container(
            name,
            Some(qp::RemoveContainerOptions {
                force,
                ..Default::default()
            }),
        )
        .await?;
    Ok(())
}

pub async fn docker_run_detached(
    container_name: &str,
    image: &str,
    project_dir: &Path,
    host_ssh_port: Option<u16>,
) -> Result<()> {
    let docker = docker_client()?;

    let binds = vec![format!("{}:/workspace", project_dir.display())];
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
    if let Some(port) = host_ssh_port {
        port_bindings.insert(
            "22/tcp".into(),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".into()),
                host_port: Some(port.to_string()),
            }]),
        );
    }

    let host_config = HostConfig {
        binds: Some(binds),
        port_bindings: if port_bindings.is_empty() {
            None
        } else {
            Some(port_bindings)
        },
        ..Default::default()
    };

    let config = ContainerCreateBody {
        image: Some(image.to_string()),
        cmd: Some(vec![
            "/bin/sh".into(),
            "-lc".into(),
            "tail -f /dev/null".into(),
        ]),
        working_dir: Some("/workspace".into()),
        host_config: Some(host_config),
        ..Default::default()
    };

    let _ = docker
        .create_container(
            Some(qp::CreateContainerOptions {
                name: Some(container_name.to_string()),
                ..Default::default()
            }),
            config,
        )
        .await?;

    docker
        .start_container(container_name, None::<qp::StartContainerOptions>)
        .await?;

    Ok(())
}

pub async fn docker_exec_shell(container_name: &str, script: &str) -> Result<()> {
    let docker = docker_client()?;
    // Try bash first
    if exec_and_wait(&docker, container_name, None, &["/bin/bash", "-lc", script]).await? {
        return Ok(());
    }
    // Fallback to sh
    if exec_and_wait(&docker, container_name, None, &["/bin/sh", "-lc", script]).await? {
        return Ok(());
    }
    Err(anyhow!("docker exec failed"))
}

pub async fn docker_exec_shell_as(container_name: &str, user: &str, script: &str) -> Result<()> {
    let docker = docker_client()?;
    // Try bash first
    if exec_and_wait(
        &docker,
        container_name,
        Some(user),
        &["/bin/bash", "-lc", script],
    )
    .await?
    {
        return Ok(());
    }
    // Fallback to sh
    if exec_and_wait(
        &docker,
        container_name,
        Some(user),
        &["/bin/sh", "-lc", script],
    )
    .await?
    {
        return Ok(());
    }
    Err(anyhow!("docker exec -u failed"))
}

pub async fn docker_exec_interactive_shell(container_name: &str) -> Result<()> {
    let docker = docker_client()?;
    if exec_interactive(&docker, container_name, None, &["/bin/bash", "-l"]).await? {
        return Ok(());
    }
    // Fallback to sh
    if exec_interactive(&docker, container_name, None, &["/bin/sh", "-l"]).await? {
        return Ok(());
    }
    Err(anyhow!("failed to attach interactive shell"))
}

async fn exec_and_wait(
    docker: &Docker,
    container_name: &str,
    user: Option<&str>,
    cmd: &[&str],
) -> Result<bool> {
    let exec = docker
        .create_exec(
            container_name,
            CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                user: user.map(|u| u.to_string()),
                ..Default::default()
            },
        )
        .await?;
    match docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                tty: false,
                ..Default::default()
            }),
        )
        .await?
    {
        StartExecResults::Detached => {}
        StartExecResults::Attached { mut output, .. } => {
            while let Some(chunk) = output.next().await {
                if let Ok(log) = chunk {
                    use bollard::container::LogOutput;
                    match log {
                        LogOutput::StdOut { message } | LogOutput::StdErr { message } => {
                            let _ = std::io::Write::write_all(&mut std::io::stdout(), &message);
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                        LogOutput::Console { message } => {
                            let _ = std::io::Write::write_all(&mut std::io::stdout(), &message);
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let inspected = docker.inspect_exec(&exec.id).await?;
    Ok(matches!(inspected.exit_code, Some(0)))
}

async fn exec_interactive(
    docker: &Docker,
    container_name: &str,
    user: Option<&str>,
    cmd: &[&str],
) -> Result<bool> {
    let _raw_mode = RawModeGuard::enable()?;
    let exec = docker
        .create_exec(
            container_name,
            CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                attach_stdin: Some(true),
                tty: Some(true),
                cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                user: user.map(|u| u.to_string()),
                ..Default::default()
            },
        )
        .await?;
    if let StartExecResults::Attached { mut output, input } = docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                tty: true,
                ..Default::default()
            }),
        )
        .await?
    {
        // Initial resize to current terminal size (best-effort)
        if let Ok((cols, rows)) = crossterm::terminal::size() {
            let _ = docker
                .resize_exec(
                    &exec.id,
                    ResizeExecOptions {
                        height: rows,
                        width: cols,
                    },
                )
                .await;
        }

        // Watch for window size changes and resize TTY
        #[cfg(unix)]
        let resize_handle = {
            let docker = docker.clone();
            let exec_id = exec.id.clone();
            tokio::spawn(async move {
                if let Ok(mut sig) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
                {
                    while sig.recv().await.is_some() {
                        if let Ok((cols, rows)) = crossterm::terminal::size() {
                            let _ = docker
                                .resize_exec(
                                    &exec_id,
                                    ResizeExecOptions {
                                        height: rows,
                                        width: cols,
                                    },
                                )
                                .await;
                        }
                    }
                }
            })
        };

        #[cfg(windows)]
        let resize_handle = {
            use tokio::time::{Duration, sleep};
            let docker = docker.clone();
            let exec_id = exec.id.clone();
            tokio::spawn(async move {
                let mut last = (0u16, 0u16);
                loop {
                    if let Ok((cols, rows)) = crossterm::terminal::size() {
                        if (cols, rows) != last {
                            let _ = docker
                                .resize_exec(
                                    &exec_id,
                                    ResizeExecOptions {
                                        height: rows,
                                        width: cols,
                                    },
                                )
                                .await;
                            last = (cols, rows);
                        }
                    }
                    sleep(Duration::from_millis(250)).await;
                }
            })
        };

        // Pipe stdout/err from container to local stdout
        let out_task = tokio::spawn(async move {
            let mut stdout = io::stdout();
            while let Some(chunk) = output.next().await {
                if let Ok(log) = chunk {
                    use bollard::container::LogOutput;
                    match log {
                        LogOutput::StdOut { message }
                        | LogOutput::StdErr { message }
                        | LogOutput::Console { message } => {
                            let _ = stdout.write_all(&message).await;
                            let _ = stdout.flush().await;
                        }
                        _ => {}
                    }
                }
            }
        });

        // Pipe local stdin to container input if available
        let in_task = tokio::spawn(async move {
            let mut input = input;
            let mut stdin = io::stdin();
            let mut buf = [0u8; 1024];
            loop {
                match stdin.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if input.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                        let _ = input.flush().await;
                    }
                    Err(_) => break,
                }
            }
        });

        let _ = tokio::try_join!(out_task, in_task);
        // Stop resize watcher
        resize_handle.abort();
    }

    let inspected = docker.inspect_exec(&exec.id).await?;
    Ok(matches!(inspected.exit_code, Some(0)))
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> Result<Self> {
        enable_raw_mode().map_err(|e| anyhow!(e))?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn create_tar_from_dir(dir: &Path) -> Result<Vec<u8>> {
    let mut ar = tar::Builder::new(Vec::<u8>::new());
    let base = dir;
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        let rel = match path.strip_prefix(base) {
            Ok(p) if p.as_os_str().is_empty() => PathBuf::from("."),
            Ok(p) => p.to_path_buf(),
            Err(_) => PathBuf::from("."),
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        if path.is_dir() {
            ar.append_dir(rel, path)?;
        } else if path.is_file() {
            ar.append_path_with_name(path, rel)?;
        }
    }
    let data = ar.into_inner()?;
    Ok(data)
}
