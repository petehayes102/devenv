use std::{
    collections::HashMap,
    future,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, bail};
use bollard::{
    Docker, body_full,
    exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults},
    models::{ContainerCreateBody, HostConfig, PortBinding},
    query_parameters as qp,
};
use bytes::Bytes;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures_util::StreamExt;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error};
use walkdir::WalkDir;

pub mod file;

pub struct DockerClient(Docker);

#[derive(Debug, Clone)]
pub struct PsItem {
    pub name: String,
    pub image: String,
    pub status: String,
}

struct RawModeGuard;

impl DockerClient {
    pub fn new() -> Result<Self> {
        let inner = Docker::connect_with_local_defaults()?;
        Ok(Self(inner))
    }

    pub async fn build_with_opts(
        &self,
        context_dir: &Path,
        tag: &str,
        pull: bool,
        no_cache: bool,
    ) -> Result<()> {
        let tar = create_tar_from_dir(context_dir)?;
        let opts = qp::BuildImageOptionsBuilder::default()
            .dockerfile("Dockerfile")
            .t(tag)
            .pull(if pull { "true" } else { "false" })
            .nocache(no_cache)
            .rm(true)
            .build();
        let body = body_full(Bytes::from(tar));
        let stream = self.0.build_image(opts, None, Some(body));
        stream
            .for_each(|msg| {
                match msg {
                    Ok(msg) => debug!("{msg:?}"),
                    Err(e) => error!("{e:?}"),
                }

                future::ready(())
            })
            .await;
        Ok(())
    }

    pub async fn ps(&self) -> Result<Vec<PsItem>> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("name".into(), vec!["devenv-".into()]);
        let containers = self
            .0
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
            out.push(PsItem {
                name,
                image: c.image.unwrap_or_default(),
                status: c.status.unwrap_or_default(),
            });
        }
        Ok(out)
    }

    pub async fn container_exists(&self, name: &str) -> Result<bool> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("name".into(), vec![name.to_string()]);
        let containers = self
            .0
            .list_containers(Some(qp::ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;
        Ok(!containers.is_empty())
    }

    pub async fn is_container_running(&self, name: &str) -> Result<bool> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("name".into(), vec![name.to_string()]);
        let containers = self
            .0
            .list_containers(Some(qp::ListContainersOptions {
                all: false,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;
        Ok(!containers.is_empty())
    }

    pub async fn start(&self, name: &str) -> Result<()> {
        self.0
            .start_container(name, None::<qp::StartContainerOptions>)
            .await?;
        Ok(())
    }

    pub async fn stop(&self, name: &str) -> Result<()> {
        self.0
            .stop_container(name, None::<qp::StopContainerOptions>)
            .await?;
        Ok(())
    }

    pub async fn remove_container(&self, name: &str, force: bool) -> Result<()> {
        self.0
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

    pub async fn run_detached(
        &self,
        container_name: &str,
        image: &str,
        project_dir: &Path,
        host_ssh_port: Option<u16>,
    ) -> Result<()> {
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
                "sleep infinity".into(),
            ]),
            working_dir: Some("/workspace".into()),
            host_config: Some(host_config),
            ..Default::default()
        };

        self.0
            .create_container(
                Some(qp::CreateContainerOptions {
                    name: Some(container_name.to_string()),
                    ..Default::default()
                }),
                config,
            )
            .await?;

        self.0
            .start_container(container_name, None::<qp::StartContainerOptions>)
            .await?;

        Ok(())
    }

    pub async fn exec_shell(&self, container_name: &str, script: &str) -> Result<()> {
        // Try bash first
        if self
            .exec_and_wait(container_name, None, &["/bin/bash", "-lc", script])
            .await?
        {
            return Ok(());
        }
        // Fallback to sh
        if self
            .exec_and_wait(container_name, None, &["/bin/sh", "-lc", script])
            .await?
        {
            return Ok(());
        }
        bail!("`docker exec` failed")
    }

    pub async fn exec_shell_as(
        &self,
        container_name: &str,
        user: &str,
        script: &str,
    ) -> Result<()> {
        // Try bash first
        if self
            .exec_and_wait(container_name, Some(user), &["/bin/bash", "-lc", script])
            .await?
        {
            return Ok(());
        }
        // Fallback to sh
        if self
            .exec_and_wait(container_name, Some(user), &["/bin/sh", "-lc", script])
            .await?
        {
            return Ok(());
        }
        bail!("`docker exec -u` failed")
    }

    pub async fn exec_interactive_shell(&self, container_name: &str) -> Result<()> {
        if self
            .exec_interactive(container_name, None, &["/bin/bash", "-l"])
            .await?
        {
            return Ok(());
        }
        // Fallback to sh
        if self
            .exec_interactive(container_name, None, &["/bin/sh", "-l"])
            .await?
        {
            return Ok(());
        }
        bail!("failed to attach interactive shell")
    }

    async fn exec_and_wait(
        &self,
        container_name: &str,
        user: Option<&str>,
        cmd: &[&str],
    ) -> Result<bool> {
        let exec = self
            .0
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
        match self
            .0
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
        let inspected = self.0.inspect_exec(&exec.id).await?;
        Ok(matches!(inspected.exit_code, Some(0)))
    }

    async fn exec_interactive(
        &self,
        container_name: &str,
        user: Option<&str>,
        cmd: &[&str],
    ) -> Result<bool> {
        let _raw_mode = RawModeGuard::enable()?;
        let exec = self
            .0
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
        if let StartExecResults::Attached {
            mut output,
            mut input,
        } = self
            .0
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
                let _ = self
                    .0
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
                let docker = self.0.clone();
                let exec_id = exec.id.clone();
                tokio::spawn(async move {
                    if let Ok(mut sig) = tokio::signal::unix::signal(
                        tokio::signal::unix::SignalKind::window_change(),
                    ) {
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
                let docker = self.0.clone();
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

        let inspected = self.0.inspect_exec(&exec.id).await?;
        Ok(matches!(inspected.exit_code, Some(0)))
    }
}

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
