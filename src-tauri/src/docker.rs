use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions, LogsOptions,
    RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::image::{CreateImageOptions, ListImagesOptions, RemoveImageOptions};
use bollard::models::{HostConfig, PortBinding, PortMap};
use bollard::network::ListNetworksOptions;
use bollard::volume::ListVolumesOptions;
use bollard::Docker;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::io::{BufReader, Read};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, State};

pub struct DockerState {
    pub client: std::sync::Arc<std::sync::Mutex<Option<Docker>>>,
}

fn get_client(docker: &DockerState) -> Result<Docker, String> {
    docker
        .client
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "Docker not connected. Start a runtime first.".to_string())
}

#[derive(Debug, Serialize, Clone)]
pub struct ContainerInfo {
    pub id: String,
    pub names: Vec<String>,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: Vec<PortInfo>,
    pub created: i64,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PortInfo {
    pub private_port: u16,
    pub public_port: Option<u16>,
    pub port_type: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ImageInfo {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub size: i64,
    pub created: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct VolumeInfo {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct NetworkInfo {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub containers: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct ContainerGroup {
    pub name: String,
    pub containers: Vec<ContainerInfo>,
}

/// Create a Docker CLI command pinned to the provider selected in the app.
fn docker_cmd(provider: crate::provider::ProviderKind) -> Command {
    let mut cmd = Command::new(crate::runtime::docker_cli_for(provider));
    if let Some(host) = crate::runtime::docker_host_for(provider) {
        cmd.env("DOCKER_HOST", host);
    }
    cmd
}

/// Build a CLI command for an exec-style operation, choosing `docker` or
/// Apple's `container` based on the active provider. For Docker the command
/// verb is `exec`/`cp`; for Apple it's the same subcommand names under `container`.
fn exec_cmd(provider: crate::provider::ProviderKind, verb: &str) -> Command {
    let mut cmd = if provider == crate::provider::ProviderKind::Apple {
        Command::new(crate::apple::container_bin())
    } else {
        docker_cmd(provider)
    };
    cmd.arg(verb);
    cmd
}

fn validate_container_id(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Invalid container ID".to_string());
    }
    Ok(())
}

fn extract_compose_project(labels: &HashMap<String, String>) -> Option<String> {
    labels.get("com.docker.compose.project").cloned()
}

fn container_from_summary(c: bollard::models::ContainerSummary) -> ContainerInfo {
    let labels = c.labels.unwrap_or_default();
    ContainerInfo {
        id: c.id.unwrap_or_default().chars().take(12).collect(),
        names: c
            .names
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.trim_start_matches('/').to_string())
            .collect(),
        image: c.image.unwrap_or_default(),
        state: c.state.unwrap_or_default(),
        status: c.status.unwrap_or_default(),
        ports: c
            .ports
            .unwrap_or_default()
            .into_iter()
            .map(|p| PortInfo {
                private_port: p.private_port,
                public_port: p.public_port,
                port_type: p.typ.map(|t| format!("{:?}", t)).unwrap_or_default(),
            })
            .collect(),
        created: c.created.unwrap_or(0),
        labels,
    }
}

#[tauri::command]
pub async fn list_containers(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
) -> Result<Vec<ContainerGroup>, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::list_containers();
    }

    let opts = ListContainersOptions::<String> {
        all: true,
        ..Default::default()
    };

    let containers = get_client(&docker)?
        .list_containers(Some(opts))
        .await
        .map_err(|e| e.to_string())?;

    let mut groups: HashMap<String, Vec<ContainerInfo>> = HashMap::new();
    let mut ungrouped: Vec<ContainerInfo> = Vec::new();

    for c in containers {
        let info = container_from_summary(c);
        if let Some(project) = extract_compose_project(&info.labels) {
            groups.entry(project).or_default().push(info);
        } else {
            ungrouped.push(info);
        }
    }

    let mut result: Vec<ContainerGroup> = groups
        .into_iter()
        .map(|(name, containers)| ContainerGroup { name, containers })
        .collect();

    result.sort_by(|a, b| a.name.cmp(&b.name));

    if !ungrouped.is_empty() {
        result.push(ContainerGroup {
            name: "Standalone".to_string(),
            containers: ungrouped,
        });
    }

    Ok(result)
}

#[tauri::command]
pub async fn list_images(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
) -> Result<Vec<ImageInfo>, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::list_images();
    }

    let opts = ListImagesOptions::<String> {
        all: false,
        ..Default::default()
    };

    let images = get_client(&docker)?
        .list_images(Some(opts))
        .await
        .map_err(|e| e.to_string())?;

    Ok(images
        .into_iter()
        .map(|i| ImageInfo {
            id: i.id.chars().skip(7).take(12).collect(),
            repo_tags: i.repo_tags,
            size: i.size,
            created: i.created,
        })
        .collect())
}

#[tauri::command]
pub async fn list_volumes(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
) -> Result<Vec<VolumeInfo>, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::list_volumes();
    }

    let opts = ListVolumesOptions::<String> {
        ..Default::default()
    };

    let response = get_client(&docker)?
        .list_volumes(Some(opts))
        .await
        .map_err(|e| e.to_string())?;

    Ok(response
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(|v| VolumeInfo {
            name: v.name,
            driver: v.driver,
            mountpoint: v.mountpoint,
            labels: v.labels,
        })
        .collect())
}

#[tauri::command]
pub async fn list_networks(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
) -> Result<Vec<NetworkInfo>, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::list_networks();
    }

    let opts = ListNetworksOptions::<String> {
        ..Default::default()
    };

    let networks = get_client(&docker)?
        .list_networks(Some(opts))
        .await
        .map_err(|e| e.to_string())?;

    Ok(networks
        .into_iter()
        .map(|n| NetworkInfo {
            id: n.id.unwrap_or_default().chars().take(12).collect(),
            name: n.name.unwrap_or_default(),
            driver: n.driver.unwrap_or_default(),
            scope: n.scope.unwrap_or_default(),
            containers: n.containers.map(|c| c.len()).unwrap_or(0),
        })
        .collect())
}

#[tauri::command]
pub async fn start_container(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
) -> Result<(), String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::start_container(&id);
    }
    get_client(&docker)?
        .start_container(&id, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_container(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
) -> Result<(), String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::stop_container(&id);
    }
    get_client(&docker)?
        .stop_container(&id, Some(StopContainerOptions { t: 10 }))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restart_container(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
) -> Result<(), String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::restart_container(&id);
    }
    get_client(&docker)?
        .restart_container(
            &id,
            Some(bollard::container::RestartContainerOptions { t: 10 }),
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_container_group(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    ids: Vec<String>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    if provider.get() == crate::provider::ProviderKind::Apple {
        return start_apple_compose_group(&ids);
    }

    let client = get_client(&docker)?;
    for id in ids {
        let running = client
            .inspect_container(&id, None::<InspectContainerOptions>)
            .await
            .ok()
            .and_then(|container| container.state)
            .and_then(|state| state.running)
            .unwrap_or(false);
        if running {
            continue;
        }
        client
            .start_container(&id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|error| format!("Could not start container {id}: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_container_group(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    ids: Vec<String>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    if provider.get() == crate::provider::ProviderKind::Apple {
        for id in ids.into_iter().rev() {
            // A group can contain already-stopped one-shot services. Ignore
            // only those; a real CLI failure still aborts the operation.
            if let Ok(container) = inspect_apple_compose_container(&id) {
                if !container.running {
                    continue;
                }
            }
            crate::apple::stop_container(&id)
                .map_err(|error| format!("Could not stop container {id}: {error}"))?;
        }
        return Ok(());
    }

    let client = get_client(&docker)?;
    for id in ids.into_iter().rev() {
        let running = client
            .inspect_container(&id, None::<InspectContainerOptions>)
            .await
            .ok()
            .and_then(|container| container.state)
            .and_then(|state| state.running)
            .unwrap_or(false);
        if !running {
            continue;
        }
        client
            .stop_container(&id, Some(StopContainerOptions { t: 10 }))
            .await
            .map_err(|error| format!("Could not stop container {id}: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_container_logs(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
    tail: Option<String>,
    timestamps: Option<bool>,
) -> Result<Vec<String>, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        // Apple Container logs have no timestamp annotation; ignore the flag.
        let t = tail.as_deref();
        return crate::apple::get_container_logs(&id, t);
    }
    use futures_util::StreamExt;

    let opts = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        tail: tail.unwrap_or_else(|| "100".to_string()),
        timestamps: timestamps.unwrap_or(false),
        ..Default::default()
    };

    let mut stream = get_client(&docker)?.logs(&id, Some(opts));
    let mut lines = Vec::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => lines.push(output.to_string()),
            Err(e) => return Err(e.to_string()),
        }
    }

    Ok(lines)
}

#[tauri::command]
pub async fn get_container_logs_since(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
    since: i64,
    timestamps: Option<bool>,
) -> Result<Vec<String>, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::get_container_logs_since(&id, since, timestamps.unwrap_or(false));
    }
    use futures_util::StreamExt;

    let opts = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        since,
        timestamps: timestamps.unwrap_or(false),
        ..Default::default()
    };

    let mut stream = get_client(&docker)?.logs(&id, Some(opts));
    let mut lines = Vec::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => lines.push(output.to_string()),
            Err(e) => return Err(e.to_string()),
        }
    }

    Ok(lines)
}

#[tauri::command]
pub async fn docker_ping(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
) -> Result<bool, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        // Probe the Apple backend by listing containers.
        return crate::apple::list_containers().map(|_| true);
    }
    get_client(&docker)?
        .ping()
        .await
        .map(|_| true)
        .map_err(|e| e.to_string())
}

// --- Container Env Vars ---

#[tauri::command]
pub async fn get_container_env(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
) -> Result<Vec<String>, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::get_container_env(&id);
    }
    let info = get_client(&docker)?
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await
        .map_err(|e| e.to_string())?;

    Ok(info.config.and_then(|c| c.env).unwrap_or_default())
}

// --- Remove ---

#[tauri::command]
pub async fn remove_container(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
    force: Option<bool>,
) -> Result<(), String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::remove_container(&id, force.unwrap_or(false));
    }
    get_client(&docker)?
        .remove_container(
            &id,
            Some(RemoveContainerOptions {
                force: force.unwrap_or(false),
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_image(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
) -> Result<(), String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::remove_image(&id);
    }
    get_client(&docker)?
        .remove_image(
            &id,
            Some(RemoveImageOptions {
                force: true,
                ..Default::default()
            }),
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn remove_volume(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    name: String,
) -> Result<(), String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::remove_volume(&name);
    }
    get_client(&docker)?
        .remove_volume(&name, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_network(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
) -> Result<(), String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::remove_network(&id);
    }
    get_client(&docker)?
        .remove_network(&id)
        .await
        .map_err(|e| e.to_string())
}

// --- Pull Image ---

#[tauri::command]
pub async fn pull_image(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    image: String,
) -> Result<(), String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::pull_image(&image);
    }
    use futures_util::StreamExt;

    let (repo, tag) = match image.split_once(':') {
        Some((r, t)) => (r.to_string(), t.to_string()),
        None => (image.clone(), "latest".to_string()),
    };

    let opts = CreateImageOptions {
        from_image: repo,
        tag,
        ..Default::default()
    };

    let mut stream = get_client(&docker)?.create_image(Some(opts), None, None);
    while let Some(result) = stream.next().await {
        result.map_err(|e| e.to_string())?;
    }

    Ok(())
}

// --- Create Container ---

#[derive(Debug, Deserialize)]
pub struct PortMapping {
    pub host: String,
    pub container: String,
}

#[derive(Debug, Deserialize)]
pub struct VolumeMapping {
    pub host: String,
    pub container: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateContainerInput {
    pub name: Option<String>,
    pub image: String,
    pub ports: Vec<PortMapping>,
    pub volumes: Vec<VolumeMapping>,
    pub env: Vec<String>,
    pub auto_start: bool,
}

#[tauri::command]
pub async fn create_container(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    input: CreateContainerInput,
) -> Result<String, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::create_container(&input);
    }
    let mut exposed_ports = HashMap::new();
    let mut port_bindings: PortMap = HashMap::new();

    for p in &input.ports {
        let host_port: u16 = p
            .host
            .parse()
            .map_err(|_| format!("Invalid host port: {}", p.host))?;
        let container_port_num: u16 = p
            .container
            .parse()
            .map_err(|_| format!("Invalid container port: {}", p.container))?;
        if host_port == 0 || container_port_num == 0 {
            return Err("Port must be between 1 and 65535".to_string());
        }
        let container_port = format!("{}/tcp", p.container);
        exposed_ports.insert(container_port.clone(), HashMap::new());
        port_bindings.insert(
            container_port,
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(p.host.clone()),
            }]),
        );
    }

    let binds: Vec<String> = input
        .volumes
        .iter()
        .map(|v| format!("{}:{}", v.host, v.container))
        .collect();

    let config = Config {
        image: Some(input.image.clone()),
        exposed_ports: Some(exposed_ports),
        env: Some(input.env.clone()),
        host_config: Some(HostConfig {
            port_bindings: Some(port_bindings),
            binds: Some(binds),
            ..Default::default()
        }),
        ..Default::default()
    };

    let opts = input.name.as_ref().map(|n| CreateContainerOptions {
        name: n.as_str(),
        platform: None,
    });

    let client = get_client(&docker)?;
    let response = client
        .create_container(opts, config)
        .await
        .map_err(|e| e.to_string())?;

    if input.auto_start {
        client
            .start_container(&response.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(response.id)
}

// --- Docker Compose ---

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct PublishedPort {
    port: u16,
    protocol: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComposePortConflict {
    pub ports: Vec<String>,
    pub provider: Option<crate::provider::ProviderKind>,
    pub project_name: Option<String>,
    pub container_count: usize,
    pub can_stop: bool,
}

#[derive(Debug)]
struct RuntimeContainer {
    id: String,
    project_name: Option<String>,
    published_ports: BTreeSet<PublishedPort>,
}

#[derive(Debug)]
struct RuntimeProject {
    provider: crate::provider::ProviderKind,
    project_name: String,
    container_ids: Vec<String>,
}

fn yaml_mapping_value<'a>(
    mapping: &'a serde_yaml::Mapping,
    key: &str,
) -> Option<&'a serde_yaml::Value> {
    mapping.get(serde_yaml::Value::String(key.to_string()))
}

fn expand_port_range(value: &str) -> Vec<u16> {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    if let Some((start, end)) = value.split_once('-') {
        let (Ok(start), Ok(end)) = (start.parse::<u16>(), end.parse::<u16>()) else {
            return Vec::new();
        };
        if start == 0 || end < start || end.saturating_sub(start) > 255 {
            return Vec::new();
        }
        return (start..=end).collect();
    }

    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .into_iter()
        .collect()
}

fn short_syntax_ports(value: &str) -> Vec<PublishedPort> {
    let (address, protocol) = value
        .rsplit_once('/')
        .filter(|(_, protocol)| matches!(*protocol, "tcp" | "udp"))
        .unwrap_or((value, "tcp"));
    let mut segments = address.rsplit(':');
    let _target = segments.next();
    let Some(published) = segments.next() else {
        // A target-only entry gets an ephemeral host port and cannot conflict
        // with a specific listener before the container is created.
        return Vec::new();
    };

    expand_port_range(published)
        .into_iter()
        .map(|port| PublishedPort {
            port,
            protocol: protocol.to_string(),
        })
        .collect()
}

fn published_ports_from_compose(config: &str) -> BTreeSet<PublishedPort> {
    let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(config) else {
        return BTreeSet::new();
    };
    let Some(services) = root
        .as_mapping()
        .and_then(|root| yaml_mapping_value(root, "services"))
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return BTreeSet::new();
    };

    let mut ports = BTreeSet::new();
    for service in services.values().filter_map(serde_yaml::Value::as_mapping) {
        let Some(entries) =
            yaml_mapping_value(service, "ports").and_then(serde_yaml::Value::as_sequence)
        else {
            continue;
        };
        for entry in entries {
            match entry {
                serde_yaml::Value::String(value) => ports.extend(short_syntax_ports(value)),
                serde_yaml::Value::Mapping(mapping) => {
                    let Some(published) = yaml_mapping_value(mapping, "published") else {
                        continue;
                    };
                    let published = match published {
                        serde_yaml::Value::String(value) => value.clone(),
                        serde_yaml::Value::Number(value) => value.to_string(),
                        _ => continue,
                    };
                    let protocol = yaml_mapping_value(mapping, "protocol")
                        .and_then(serde_yaml::Value::as_str)
                        .filter(|protocol| matches!(*protocol, "tcp" | "udp"))
                        .unwrap_or("tcp");
                    ports.extend(expand_port_range(&published).into_iter().map(|port| {
                        PublishedPort {
                            port,
                            protocol: protocol.to_string(),
                        }
                    }));
                }
                _ => {}
            }
        }
    }
    ports
}

fn port_is_in_use(port: &PublishedPort) -> bool {
    let selector = if port.protocol == "udp" {
        format!("-iUDP:{}", port.port)
    } else {
        format!("-iTCP:{}", port.port)
    };
    let mut command = Command::new("/usr/sbin/lsof");
    command.args(["-nP", &selector]);
    if port.protocol == "tcp" {
        command.arg("-sTCP:LISTEN");
    }
    command
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
}

fn compose_published_ports(mocker: Option<&Path>, file_path: &str) -> BTreeSet<PublishedPort> {
    let resolved = mocker.and_then(|mocker| {
        Command::new(mocker)
            .args(["compose", "-f", file_path, "config"])
            .stdin(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    });
    let config = resolved.or_else(|| std::fs::read_to_string(file_path).ok());
    config
        .as_deref()
        .map(published_ports_from_compose)
        .unwrap_or_default()
}

fn mocker_port_conflicts(mocker: &Path, file_path: &str) -> Vec<PublishedPort> {
    compose_published_ports(Some(mocker), file_path)
        .into_iter()
        .filter(port_is_in_use)
        .collect()
}

fn runtime_containers(provider: crate::provider::ProviderKind) -> Vec<RuntimeContainer> {
    let ids = match docker_cmd(provider).args(["ps", "-q"]).output() {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        _ => return Vec::new(),
    };
    if ids.is_empty() {
        return Vec::new();
    }

    let output = match docker_cmd(provider).arg("inspect").args(&ids).output() {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let Ok(items) = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout) else {
        return Vec::new();
    };

    items
        .into_iter()
        .filter_map(|item| {
            let id = item.get("Id")?.as_str()?.to_string();
            let labels = item
                .pointer("/Config/Labels")
                .and_then(serde_json::Value::as_object);
            let project_name = labels
                .and_then(|labels| labels.get("com.docker.compose.project"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);

            let mut published_ports = BTreeSet::new();
            if let Some(bindings) = item
                .pointer("/NetworkSettings/Ports")
                .and_then(serde_json::Value::as_object)
            {
                for (target, host_bindings) in bindings {
                    let protocol = target
                        .rsplit_once('/')
                        .map(|(_, protocol)| protocol)
                        .unwrap_or("tcp");
                    for binding in host_bindings
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_object)
                    {
                        if let Some(port) = binding
                            .get("HostPort")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|port| port.parse::<u16>().ok())
                        {
                            published_ports.insert(PublishedPort {
                                port,
                                protocol: protocol.to_string(),
                            });
                        }
                    }
                }
            }

            Some(RuntimeContainer {
                id,
                project_name,
                published_ports,
            })
        })
        .collect()
}

fn display_port(port: &PublishedPort) -> String {
    if port.protocol == "tcp" {
        port.port.to_string()
    } else {
        format!("{}/{}", port.port, port.protocol)
    }
}

fn detect_compose_conflicts(file_path: &str) -> (Vec<ComposePortConflict>, Vec<RuntimeProject>) {
    let requested = compose_published_ports(crate::runtime::mocker_cli().as_deref(), file_path);
    let occupied = requested
        .into_iter()
        .filter(port_is_in_use)
        .collect::<BTreeSet<_>>();
    if occupied.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut conflicts = Vec::new();
    let mut projects = Vec::new();
    let mut claimed_ports = BTreeSet::new();

    for provider in [
        crate::provider::ProviderKind::Colima,
        crate::provider::ProviderKind::Docker,
    ] {
        let containers = runtime_containers(provider);
        let mut project_names = containers
            .iter()
            .filter(|container| {
                !container.published_ports.is_disjoint(&occupied)
                    && container.project_name.is_some()
            })
            .filter_map(|container| container.project_name.clone())
            .collect::<Vec<_>>();
        project_names.sort();
        project_names.dedup();

        for project_name in project_names {
            let project_containers = containers
                .iter()
                .filter(|container| container.project_name.as_deref() == Some(&project_name))
                .collect::<Vec<_>>();
            let ports = project_containers
                .iter()
                .flat_map(|container| container.published_ports.iter())
                .filter(|port| occupied.contains(*port))
                .cloned()
                .collect::<BTreeSet<_>>();
            claimed_ports.extend(ports.iter().cloned());
            conflicts.push(ComposePortConflict {
                ports: ports.iter().map(display_port).collect(),
                provider: Some(provider),
                project_name: Some(project_name.clone()),
                container_count: project_containers.len(),
                can_stop: true,
            });
            projects.push(RuntimeProject {
                provider,
                project_name,
                container_ids: project_containers
                    .iter()
                    .map(|container| container.id.clone())
                    .collect(),
            });
        }
    }

    let unknown = occupied
        .difference(&claimed_ports)
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        conflicts.push(ComposePortConflict {
            ports: unknown.iter().map(display_port).collect(),
            provider: None,
            project_name: None,
            container_count: 0,
            can_stop: false,
        });
    }

    (conflicts, projects)
}

#[tauri::command]
pub async fn inspect_compose_conflicts(
    provider: State<'_, crate::provider::ProviderState>,
    file_path: String,
) -> Result<Vec<ComposePortConflict>, String> {
    if provider.get() != crate::provider::ProviderKind::Apple {
        return Ok(Vec::new());
    }
    validate_compose_file(&file_path)?;
    Ok(detect_compose_conflicts(&file_path).0)
}

#[tauri::command]
pub async fn stop_conflicting_compose_projects(
    provider: State<'_, crate::provider::ProviderState>,
    file_path: String,
) -> Result<(), String> {
    if provider.get() != crate::provider::ProviderKind::Apple {
        return Err(
            "Conflict resolution is only available for Apple Container Compose.".to_string(),
        );
    }
    validate_compose_file(&file_path)?;
    let (conflicts, projects) = detect_compose_conflicts(&file_path);
    if conflicts.iter().any(|conflict| !conflict.can_stop) {
        return Err(
            "One or more ports are owned by a non-Compose process. Close it or change the Compose ports, then try again."
                .to_string(),
        );
    }

    for project in projects {
        if project.container_ids.is_empty() {
            continue;
        }
        let output = docker_cmd(project.provider)
            .arg("stop")
            .args(&project.container_ids)
            .output()
            .map_err(|error| {
                format!(
                    "Could not stop Compose project {}: {error}",
                    project.project_name
                )
            })?;
        if !output.status.success() {
            let details = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "Could not stop Compose project {}: {}",
                project.project_name,
                details.trim()
            ));
        }
    }
    Ok(())
}

fn mocker_project_exists(mocker: &Path, file_path: &str) -> bool {
    Command::new(mocker)
        .args(["compose", "-f", file_path, "ps", "-q"])
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.trim_ascii().is_empty())
}

const COMPOSE_HOSTS_BEGIN: &str = "# docker-tray compose hosts begin";
const COMPOSE_HOSTS_END: &str = "# docker-tray compose hosts end";

#[derive(Debug, Clone, PartialEq, Eq)]
struct MockerComposeContainer {
    id: String,
    service: String,
    network: String,
    ip: String,
    running: bool,
}

#[derive(Debug, Default)]
struct ComposeServicePolicy {
    dependencies: Vec<String>,
    restartable: bool,
}

fn command_details(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|part| !part.is_empty())
        .unwrap_or("unknown error")
        .to_string();
    details
}

fn inspect_apple_compose_container(id: &str) -> Result<MockerComposeContainer, String> {
    // Mocker persists the address assigned when the container was first
    // created, but Apple may assign a different address after a stop/start.
    // The Apple runtime is therefore authoritative for live network state.
    let output = Command::new(crate::apple::container_bin())
        .args(["inspect", id])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not inspect Apple Compose container {id}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not inspect Apple Compose container {id}: {}",
            command_details(&output)
        ));
    }

    let value = serde_json::from_slice::<Value>(&output.stdout)
        .map_err(|error| format!("Invalid Apple inspect response for {id}: {error}"))?;
    let item = value
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(&value);
    apple_compose_container_from_inspect(item, id)
}

fn apple_compose_container_from_inspect(
    item: &Value,
    fallback_id: &str,
) -> Result<MockerComposeContainer, String> {
    let string_at = |pointer: &str| {
        item.pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let inspected_id = string_at("/id");
    let service = string_at("/configuration/labels/com.mocker.compose.service");
    let configured_network = string_at("/configuration/labels/com.mocker.compose.network");
    let attachments = item.pointer("/status/networks").and_then(Value::as_array);
    let attachment = attachments
        .and_then(|networks| {
            networks.iter().find(|network| {
                configured_network.is_empty()
                    || network.pointer("/network").and_then(Value::as_str)
                        == Some(configured_network.as_str())
            })
        })
        .or_else(|| attachments.and_then(|networks| networks.first()));
    let network = attachment
        .and_then(|network| network.pointer("/network"))
        .and_then(Value::as_str)
        .unwrap_or(&configured_network)
        .to_string();
    let ip = attachment
        .and_then(|network| network.pointer("/ipv4Address"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string();
    if service.is_empty() {
        return Err(format!(
            "Apple Compose container {fallback_id} has no Mocker service label"
        ));
    }
    Ok(MockerComposeContainer {
        id: if inspected_id.is_empty() {
            fallback_id.to_string()
        } else {
            inspected_id
        },
        service,
        network,
        ip,
        running: item
            .pointer("/status/state")
            .and_then(Value::as_str)
            .is_some_and(|state| state.eq_ignore_ascii_case("running")),
    })
}

fn mocker_compose_containers(
    mocker: &Path,
    file_path: &str,
) -> Result<Vec<MockerComposeContainer>, String> {
    let output = Command::new(mocker)
        .args(["compose", "-f", file_path, "ps", "-q"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not list Apple Compose containers: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not list Apple Compose containers: {}",
            command_details(&output)
        ));
    }

    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(|id| {
            let mut last_error = None;
            for attempt in 0..10 {
                match inspect_apple_compose_container(id) {
                    Ok(container) => return Ok(container),
                    Err(error) => last_error = Some(error),
                }
                if attempt < 9 {
                    thread::sleep(Duration::from_millis(150));
                }
            }
            Err(last_error.unwrap_or_else(|| format!("Could not inspect {id}")))
        })
        .collect()
}

fn restart_policy_enabled(value: Option<&serde_yaml::Value>) -> bool {
    match value {
        Some(serde_yaml::Value::String(policy)) => {
            matches!(
                policy.to_ascii_lowercase().as_str(),
                "always" | "unless-stopped" | "on-failure"
            ) || policy.to_ascii_lowercase().starts_with("on-failure:")
        }
        Some(serde_yaml::Value::Bool(enabled)) => *enabled,
        _ => false,
    }
}

fn compose_service_policies(
    file_path: &str,
) -> Result<(Vec<String>, HashMap<String, ComposeServicePolicy>), String> {
    let source = fs::read_to_string(file_path)
        .map_err(|error| format!("Could not read Compose file: {error}"))?;
    let root = serde_yaml::from_str::<serde_yaml::Value>(&source)
        .map_err(|error| format!("Invalid Compose YAML: {error}"))?;
    let services = root
        .as_mapping()
        .and_then(|root| yaml_mapping_value(root, "services"))
        .and_then(serde_yaml::Value::as_mapping)
        .ok_or_else(|| "Compose file has no services section".to_string())?;

    let mut declared_order = Vec::new();
    let mut policies = HashMap::new();
    for (name, value) in services {
        let Some(name) = name.as_str() else {
            continue;
        };
        let Some(service) = value.as_mapping() else {
            continue;
        };
        let dependencies = match yaml_mapping_value(service, "depends_on") {
            Some(serde_yaml::Value::Sequence(items)) => items
                .iter()
                .filter_map(serde_yaml::Value::as_str)
                .map(str::to_string)
                .collect(),
            Some(serde_yaml::Value::Mapping(items)) => items
                .keys()
                .filter_map(serde_yaml::Value::as_str)
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        };
        declared_order.push(name.to_string());
        policies.insert(
            name.to_string(),
            ComposeServicePolicy {
                dependencies,
                restartable: restart_policy_enabled(yaml_mapping_value(service, "restart")),
            },
        );
    }

    // Preserve declaration order where possible, but never place a service
    // before one of its Compose dependencies.
    let mut ordered = Vec::new();
    let mut remaining = declared_order;
    while !remaining.is_empty() {
        let before = remaining.len();
        let remaining_names = remaining.iter().cloned().collect::<HashSet<_>>();
        let mut next = Vec::new();
        for name in remaining {
            let blocked = policies.get(&name).is_some_and(|policy| {
                policy
                    .dependencies
                    .iter()
                    .any(|dependency| remaining_names.contains(dependency))
            });
            if blocked {
                next.push(name);
            } else {
                ordered.push(name);
            }
        }
        if next.len() == before {
            // Cycles are invalid for dependency ordering, but Mocker already
            // accepted the file. Retain deterministic declaration order.
            ordered.extend(next);
            break;
        }
        remaining = next;
    }

    Ok((ordered, policies))
}

fn hosts_for_container(
    target: &MockerComposeContainer,
    containers: &[MockerComposeContainer],
) -> (Vec<String>, HashSet<String>) {
    let mut lines = Vec::new();
    let mut managed = HashSet::new();
    for peer in containers {
        let mut aliases = vec![peer.service.clone()];
        if peer.id != peer.service {
            aliases.push(peer.id.clone());
        }
        aliases.sort();
        aliases.dedup();
        managed.extend(aliases.iter().cloned());
        if peer.ip.parse::<IpAddr>().is_ok()
            && (target.network.is_empty()
                || peer.network.is_empty()
                || target.network == peer.network)
        {
            lines.push(format!("{} {}", peer.ip, aliases.join(" ")));
        }
    }
    (lines, managed)
}

fn rewrite_hosts(current: &str, lines: &[String], managed: &HashSet<String>) -> String {
    let mut kept = Vec::new();
    let mut inside_managed_block = false;
    for line in current.lines() {
        if line.trim() == COMPOSE_HOSTS_BEGIN {
            inside_managed_block = true;
            continue;
        }
        if line.trim() == COMPOSE_HOSTS_END {
            inside_managed_block = false;
            continue;
        }
        if inside_managed_block {
            continue;
        }
        let mentions_managed_name = line
            .split_whitespace()
            .skip(1)
            .any(|word| managed.contains(word));
        if !mentions_managed_name {
            kept.push(line.to_string());
        }
    }
    while kept.last().is_some_and(|line| line.is_empty()) {
        kept.pop();
    }
    kept.push(COMPOSE_HOSTS_BEGIN.to_string());
    kept.extend(lines.iter().cloned());
    kept.push(COMPOSE_HOSTS_END.to_string());
    format!("{}\n", kept.join("\n"))
}

fn inject_hosts_with_shell(
    container_id: &str,
    lines: &[String],
    managed: &HashSet<String>,
) -> bool {
    const SCRIPT: &str = r##"tmp="$(mktemp)" || exit 1
managed=" $1 "
awk -v managed="$managed" '
$0 == "# docker-tray compose hosts begin" { skip=1; next }
$0 == "# docker-tray compose hosts end" { skip=0; next }
skip { next }
{
  for (i=2; i<=NF; i++) {
    if (index(managed, " " $i " ")) next
  }
  print
}' /etc/hosts > "$tmp" || exit 1
cat >> "$tmp" || exit 1
cat "$tmp" > /etc/hosts
result=$?
rm -f "$tmp"
exit $result"##;

    let mut names = managed.iter().cloned().collect::<Vec<_>>();
    names.sort();
    let mut child = match Command::new(crate::apple::container_bin())
        .args([
            "exec",
            "-i",
            "--user",
            "0",
            container_id,
            "sh",
            "-c",
            SCRIPT,
            "docker-tray",
            &names.join(" "),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let payload = format!(
        "{COMPOSE_HOSTS_BEGIN}\n{}\n{COMPOSE_HOSTS_END}\n",
        lines.join("\n")
    );
    if child
        .stdin
        .take()
        .is_none_or(|mut stdin| stdin.write_all(payload.as_bytes()).is_err())
    {
        let _ = child.kill();
        return false;
    }
    child.wait().is_ok_and(|status| status.success())
}

fn compose_hosts_temp_dir() -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..10 {
        let path = std::env::temp_dir().join(format!(
            "docker-tray-compose-hosts-{}-{stamp}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not create hosts workspace: {error}")),
        }
    }
    Err("Could not create a unique hosts workspace".to_string())
}

fn inject_hosts_with_copy(container_id: &str, lines: &[String], managed: &HashSet<String>) -> bool {
    let Ok(directory) = compose_hosts_temp_dir() else {
        return false;
    };
    let local_hosts = directory.join("hosts");
    let copied_out = Command::new(crate::apple::container_bin())
        .args(["cp", &format!("{container_id}:/etc/hosts")])
        .arg(&local_hosts)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let success = copied_out
        && fs::read_to_string(&local_hosts)
            .map(|current| rewrite_hosts(&current, lines, managed))
            .and_then(|updated| fs::write(&local_hosts, updated))
            .is_ok()
        && Command::new(crate::apple::container_bin())
            .arg("cp")
            .arg(&local_hosts)
            .arg(format!("{container_id}:/etc/hosts"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    let _ = fs::remove_dir_all(&directory);
    success
}

fn inject_mocker_service_hosts(
    target: &MockerComposeContainer,
    containers: &[MockerComposeContainer],
) -> bool {
    let (lines, managed) = hosts_for_container(target, containers);
    if lines.is_empty() {
        return true;
    }
    inject_hosts_with_shell(&target.id, &lines, &managed)
        || inject_hosts_with_copy(&target.id, &lines, &managed)
}

fn start_apple_container(container_id: &str) -> Result<(), String> {
    let output = Command::new(crate::apple::container_bin())
        .args(["start", container_id])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not restart {container_id}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not restart {container_id}: {}",
            command_details(&output)
        ))
    }
}

fn start_and_inject_apple_compose_container(
    index: usize,
    containers: &mut [MockerComposeContainer],
) -> Result<(), String> {
    let id = containers[index].id.clone();
    let service = containers[index].service.clone();
    start_apple_container(&id)?;

    // A restarted Apple container receives a fresh network attachment. Wait
    // for that live address instead of reusing Mocker's persisted metadata.
    for _ in 0..20 {
        if let Ok(current) = inspect_apple_compose_container(&id) {
            containers[index] = current;
        }
        if containers[index].running
            && containers[index].ip.parse::<IpAddr>().is_ok()
            && inject_mocker_service_hosts(&containers[index], containers)
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }

    Err(format!(
        "Apple Compose started, but service discovery could not be configured in {service}"
    ))
}

fn refresh_live_apple_group_hosts(containers: &mut [MockerComposeContainer]) -> Result<(), String> {
    for container in containers.iter_mut() {
        if let Ok(current) = inspect_apple_compose_container(&container.id) {
            *container = current;
        }
    }
    for index in 0..containers.len() {
        if containers[index].running && !inject_mocker_service_hosts(&containers[index], containers)
        {
            return Err(format!(
                "Could not configure Apple Compose service discovery in {}",
                containers[index].service
            ));
        }
    }
    Ok(())
}

fn start_apple_compose_group(ids: &[String]) -> Result<(), String> {
    let mut containers = ids
        .iter()
        .map(|id| inspect_apple_compose_container(id))
        .collect::<Result<Vec<_>, _>>()?;
    let start_targets = containers
        .iter()
        .enumerate()
        .filter(|(_, container)| !container.running)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    // Start the whole group first so Apple can allocate every live address.
    // This keeps the window before hostname injection short for all services.
    for index in &start_targets {
        start_apple_container(&containers[*index].id)?;
    }

    let mut observed_running = HashSet::new();
    for index in &start_targets {
        for _ in 0..20 {
            if let Ok(current) = inspect_apple_compose_container(&containers[*index].id) {
                containers[*index] = current;
            }
            if containers[*index].running && containers[*index].ip.parse::<IpAddr>().is_ok() {
                observed_running.insert(*index);
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
    }

    refresh_live_apple_group_hosts(&mut containers)?;

    // A dependency may have exited before the first full host map landed.
    // Retry only services that were observed running; fast one-shot jobs that
    // completed normally are left stopped.
    for index in observed_running {
        if let Ok(current) = inspect_apple_compose_container(&containers[index].id) {
            containers[index] = current;
        }
        if !containers[index].running {
            start_and_inject_apple_compose_container(index, &mut containers)?;
        }
    }

    // Retried services receive new IPs, so propagate the final map once more.
    refresh_live_apple_group_hosts(&mut containers)
}

fn repair_mocker_service_discovery(
    mocker: &Path,
    file_path: &str,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let (service_order, policies) = compose_service_policies(file_path)?;
    let mut containers = mocker_compose_containers(mocker, file_path)?;
    let order = service_order
        .iter()
        .enumerate()
        .map(|(index, service)| (service.as_str(), index))
        .collect::<HashMap<_, _>>();
    containers.sort_by_key(|container| {
        order
            .get(container.service.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });

    let _ = app.emit(
        "compose-progress",
        "Tool Service-discovery Configuring container hostnames",
    );

    // Patch containers that survived Mocker's initial startup first. This also
    // removes stale mappings left by a previous project run.
    for target in containers.iter().filter(|container| container.running) {
        let _ = app.emit(
            "compose-progress",
            format!("Container {} Configuring service discovery", target.service),
        );
        if !inject_mocker_service_hosts(target, &containers) {
            return Err(format!(
                "Apple Compose started, but service discovery could not be configured in {}",
                target.service
            ));
        }
        let _ = app.emit(
            "compose-progress",
            format!("Container {} Service discovery ready", target.service),
        );
    }

    // Apple Container does not enforce Mocker's stored restart policy. Only
    // revive services whose Compose policy explicitly asks for a restart;
    // completed one-shot jobs must remain stopped.
    let restart_targets = containers
        .iter()
        .filter(|container| {
            !container.running
                && policies
                    .get(&container.service)
                    .is_some_and(|policy| policy.restartable)
        })
        .map(|container| container.service.clone())
        .collect::<Vec<_>>();

    for service in &restart_targets {
        let Some(index) = containers
            .iter()
            .position(|container| &container.service == service)
        else {
            continue;
        };
        let _ = app.emit(
            "compose-progress",
            format!("Container {service} Restarting after network repair"),
        );
        start_and_inject_apple_compose_container(index, &mut containers)?;
    }

    // Refresh every live member once more now that restartable dependencies
    // are back, then retry a service if it exited during the repair window.
    for index in 0..containers.len() {
        if let Ok(current) = inspect_apple_compose_container(&containers[index].id) {
            containers[index] = current;
        }
        if containers[index].running
            && !inject_mocker_service_hosts(&containers[index], &containers)
        {
            return Err(format!(
                "Apple Compose started, but service discovery could not be refreshed in {}",
                containers[index].service
            ));
        }
    }
    let final_restart_targets = containers
        .iter()
        .filter(|container| {
            !container.running
                && policies
                    .get(&container.service)
                    .is_some_and(|policy| policy.restartable)
        })
        .map(|container| container.service.clone())
        .collect::<Vec<_>>();
    for service in &final_restart_targets {
        let Some(index) = containers
            .iter()
            .position(|container| &container.service == service)
        else {
            continue;
        };
        start_and_inject_apple_compose_container(index, &mut containers)?;
        let _ = app.emit(
            "compose-progress",
            format!("Container {service} Service discovery ready"),
        );
    }

    // A last restart may have changed an address. Propagate the final map to
    // every live peer so no stale first-match entry remains.
    for index in 0..containers.len() {
        if let Ok(current) = inspect_apple_compose_container(&containers[index].id) {
            containers[index] = current;
        }
        if containers[index].running
            && !inject_mocker_service_hosts(&containers[index], &containers)
        {
            return Err(format!(
                "Apple Compose started, but final service discovery refresh failed in {}",
                containers[index].service
            ));
        }
    }

    let _ = app.emit("compose-progress", "Tool Service-discovery Ready");
    Ok(())
}

fn port_conflict_message(ports: &[PublishedPort]) -> String {
    let ports = ports
        .iter()
        .map(|port| {
            if port.protocol == "tcp" {
                port.port.to_string()
            } else {
                format!("{}/{}", port.port, port.protocol)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Host ports already in use: {ports}. Another runtime or process is still publishing them. Stop the conflicting Docker/Colima stack or change the host ports in the Compose file, then try again."
    )
}

fn validate_compose_file(file_path: &str) -> Result<(), String> {
    let path = Path::new(file_path);
    if !path.is_file() {
        return Err(format!("File not found: {file_path}"));
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("yml" | "yaml") => Ok(()),
        _ => Err("File must have .yml or .yaml extension".to_string()),
    }
}

fn compose_failure_message(status: std::process::ExitStatus, stderr: &str, stdout: &str) -> String {
    let details = [stderr.trim(), stdout.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = details.to_ascii_lowercase();
    if normalized.contains("address already in use") || normalized.contains("errno: 48") {
        return "A host port required by this Compose project is already in use. Stop the conflicting runtime or process, or change the host port in the Compose file, then try again."
            .to_string();
    }
    if details.is_empty() {
        return format!("Compose failed with status {status}");
    }

    const MAX_ERROR_CHARS: usize = 1200;
    if details.chars().count() <= MAX_ERROR_CHARS {
        details
    } else {
        let tail = details
            .chars()
            .rev()
            .take(MAX_ERROR_CHARS)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!("…{tail}")
    }
}

fn clean_terminal_fragment(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut cleaned = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for control in chars.by_ref() {
                    if ('@'..='~').contains(&control) {
                        break;
                    }
                }
            }
            continue;
        }
        if character == '\u{8}' {
            cleaned.pop();
        } else if !character.is_control() || character == '\t' {
            cleaned.push(character);
        }
    }
    cleaned.trim().to_string()
}

fn append_captured(captured: &mut String, fragment: &str) {
    const MAX_CAPTURE_BYTES: usize = 256 * 1024;
    captured.push_str(fragment);
    captured.push('\n');
    if captured.len() > MAX_CAPTURE_BYTES {
        let mut split = captured.len() - (MAX_CAPTURE_BYTES / 2);
        while !captured.is_char_boundary(split) {
            split += 1;
        }
        captured.drain(..split);
    }
}

fn stream_compose_pipe<R: Read + Send + 'static>(
    pipe: R,
    app: tauri::AppHandle,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut captured = String::new();
        let mut pending = Vec::new();
        let mut chunk = [0_u8; 4096];
        let mut last_emitted = String::new();

        let emit_fragment = |bytes: &[u8], captured: &mut String, last_emitted: &mut String| {
            let fragment = clean_terminal_fragment(bytes);
            if fragment.is_empty() {
                return;
            }
            append_captured(captured, &fragment);
            if fragment != *last_emitted {
                let _ = app.emit("compose-progress", &fragment);
                *last_emitted = fragment;
            }
        };

        loop {
            let read = match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            for byte in &chunk[..read] {
                if matches!(*byte, b'\r' | b'\n') {
                    emit_fragment(&pending, &mut captured, &mut last_emitted);
                    pending.clear();
                } else {
                    pending.push(*byte);
                }
            }
        }
        emit_fragment(&pending, &mut captured, &mut last_emitted);
        captured
    })
}

fn wait_for_compose(mut child: Child, app: &tauri::AppHandle) -> Result<String, String> {
    let stdout = child
        .stdout
        .take()
        .map(|pipe| stream_compose_pipe(pipe, app.clone()));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| stream_compose_pipe(pipe, app.clone()));

    let status = child.wait().map_err(|error| error.to_string())?;
    let stdout = stdout
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    if !status.success() {
        let _ = app.emit("compose-progress", "error");
        return Err(compose_failure_message(status, &stderr, &stdout));
    }

    Ok(stdout)
}

#[tauri::command]
pub async fn compose_up(
    provider: State<'_, crate::provider::ProviderState>,
    file_path: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    validate_compose_file(&file_path)?;

    let selected_provider = provider.get();
    let mut apple_mocker = None;
    let child = if selected_provider == crate::provider::ProviderKind::Apple {
        let installing = crate::runtime::mocker_cli().is_none();
        if installing {
            let _ = app.emit("compose-progress", "Tool Mocker Installing with Homebrew");
        }
        let mocker = match crate::runtime::ensure_mocker() {
            Ok(mocker) => mocker,
            Err(error) => {
                let _ = app.emit("compose-progress", "Tool Mocker Installation failed");
                let _ = app.emit("compose-progress", "error");
                return Err(error);
            }
        };
        if installing {
            let _ = app.emit("compose-progress", "Tool Mocker Installed");
        } else {
            let _ = app.emit("compose-progress", "Tool Mocker Ready");
        }
        let _ = app.emit(
            "compose-progress",
            "Tool Compose Checking port availability",
        );
        // An existing Mocker project may legitimately own its published ports;
        // let Mocker reconcile it instead of flagging its own listeners.
        let conflicts = if mocker_project_exists(&mocker, &file_path) {
            Vec::new()
        } else {
            mocker_port_conflicts(&mocker, &file_path)
        };
        if !conflicts.is_empty() {
            let ports = conflicts
                .iter()
                .map(|port| port.port.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let _ = app.emit(
                "compose-progress",
                format!("Tool Ports Failed: already in use ({ports})"),
            );
            let _ = app.emit("compose-progress", "error");
            return Err(port_conflict_message(&conflicts));
        }
        let _ = app.emit("compose-progress", "Tool Compose Ports available");
        let _ = app.emit("compose-progress", "Tool Compose Starting services");
        apple_mocker = Some(mocker.clone());
        Command::new(&mocker)
            .args(["compose", "-f", &file_path, "up", "-d"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                let _ = app.emit("compose-progress", "error");
                format!("Could not start Mocker: {error}")
            })?
    } else {
        // Docker and Colima retain the native Compose path and legacy fallback.
        let _ = app.emit("compose-progress", "Tool Compose Starting services");
        docker_cmd(selected_provider)
            .args(["compose", "-f", &file_path, "up", "-d"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .or_else(|_| {
                let mut fallback = Command::new("docker-compose");
                if let Some(host) = crate::runtime::docker_host_for(selected_provider) {
                    fallback.env("DOCKER_HOST", host);
                }
                fallback
                    .args(["-f", &file_path, "up", "-d"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
            })
            .map_err(|error| error.to_string())?
    };

    let output = wait_for_compose(child, &app)?;
    if let Some(mocker) = apple_mocker {
        if let Err(error) = repair_mocker_service_discovery(&mocker, &file_path, &app) {
            let _ = app.emit("compose-progress", "Tool Service-discovery Failed");
            let _ = app.emit("compose-progress", "error");
            return Err(error);
        }
    }
    let _ = app.emit("compose-progress", "done");
    Ok(output)
}

#[cfg(test)]
mod compose_tests {
    use super::{
        apple_compose_container_from_inspect, clean_terminal_fragment, hosts_for_container,
        published_ports_from_compose, restart_policy_enabled, rewrite_hosts,
        MockerComposeContainer, PublishedPort, COMPOSE_HOSTS_BEGIN, COMPOSE_HOSTS_END,
    };
    use std::collections::{BTreeSet, HashSet};

    #[test]
    fn extracts_published_ports_from_short_and_long_syntax() {
        let config = r#"
services:
  web:
    ports:
      - "127.0.0.1:8080:80"
      - "9090:90/udp"
      - "10000-10002:20000-20002"
      - "3000"
  api:
    ports:
      - target: 4000
        published: 4400
        protocol: tcp
"#;

        let actual = published_ports_from_compose(config);
        let expected = [
            (4400, "tcp"),
            (8080, "tcp"),
            (9090, "udp"),
            (10000, "tcp"),
            (10001, "tcp"),
            (10002, "tcp"),
        ]
        .into_iter()
        .map(|(port, protocol)| PublishedPort {
            port,
            protocol: protocol.to_string(),
        })
        .collect::<BTreeSet<_>>();

        assert_eq!(actual, expected);
    }

    #[test]
    fn cleans_terminal_progress_fragments() {
        assert_eq!(
            clean_terminal_fragment(b"\x1b[36m[4/6] Fetching init image 31%\x1b[0m"),
            "[4/6] Fetching init image 31%"
        );
        assert_eq!(clean_terminal_fragment(b"12%\x08\x0831%"), "131%");
    }

    #[test]
    fn replaces_stale_compose_host_entries_idempotently() {
        let current = format!(
            "127.0.0.1 localhost\n192.168.65.2 kafka\n{COMPOSE_HOSTS_BEGIN}\n192.168.65.3 redis\n{COMPOSE_HOSTS_END}\n"
        );
        let lines = vec![
            "192.168.65.8 kafka demo-kafka-1".to_string(),
            "192.168.65.9 redis demo-redis-1".to_string(),
        ];
        let managed = ["kafka", "demo-kafka-1", "redis", "demo-redis-1"]
            .into_iter()
            .map(str::to_string)
            .collect::<HashSet<_>>();

        let rewritten = rewrite_hosts(&current, &lines, &managed);
        assert_eq!(rewritten.matches(" kafka ").count(), 1);
        assert_eq!(rewritten.matches(COMPOSE_HOSTS_BEGIN).count(), 1);
        assert!(rewritten.contains("127.0.0.1 localhost"));
        assert!(rewritten.contains("192.168.65.9 redis demo-redis-1"));
    }

    #[test]
    fn includes_self_and_peer_aliases_on_the_shared_network() {
        let kafka = MockerComposeContainer {
            id: "demo-kafka-1".to_string(),
            service: "kafka".to_string(),
            network: "demo-network".to_string(),
            ip: "192.168.65.3".to_string(),
            running: true,
        };
        let registry = MockerComposeContainer {
            id: "demo-schema-registry-1".to_string(),
            service: "schema-registry".to_string(),
            network: "demo-network".to_string(),
            ip: "192.168.65.4".to_string(),
            running: true,
        };

        let (lines, managed) = hosts_for_container(&kafka, &[kafka.clone(), registry.clone()]);
        assert!(lines.contains(&"192.168.65.3 demo-kafka-1 kafka".to_string()));
        assert!(lines.contains(&"192.168.65.4 demo-schema-registry-1 schema-registry".to_string()));
        assert!(managed.contains("kafka"));
        assert!(managed.contains("schema-registry"));
    }

    #[test]
    fn recognizes_only_compose_restart_policies() {
        assert!(restart_policy_enabled(Some(&serde_yaml::Value::String(
            "always".to_string()
        ))));
        assert!(restart_policy_enabled(Some(&serde_yaml::Value::String(
            "on-failure:3".to_string()
        ))));
        assert!(!restart_policy_enabled(Some(&serde_yaml::Value::String(
            "no".to_string()
        ))));
        assert!(!restart_policy_enabled(None));
    }

    #[test]
    fn reads_live_apple_address_instead_of_stale_mocker_metadata() {
        let inspect = serde_json::json!({
            "id": "demo-kafka-1",
            "configuration": {
                "labels": {
                    "com.mocker.compose.network": "demo-network",
                    "com.mocker.compose.service": "kafka"
                }
            },
            "status": {
                "state": "running",
                "networks": [{
                    "network": "demo-network",
                    "ipv4Address": "192.168.65.11/24"
                }]
            }
        });

        let container = apple_compose_container_from_inspect(&inspect, "fallback").unwrap();
        assert_eq!(container.ip, "192.168.65.11");
        assert_eq!(container.service, "kafka");
        assert!(container.running);
    }
}

// --- Mount info ---

#[derive(Debug, Serialize, Clone)]
pub struct MountInfo {
    pub mount_type: String,
    pub source: String,
    pub destination: String,
    pub mode: String,
    pub rw: bool,
}

#[tauri::command]
pub async fn get_container_mounts(
    provider: State<'_, crate::provider::ProviderState>,
    docker: State<'_, DockerState>,
    id: String,
) -> Result<Vec<MountInfo>, String> {
    if provider.get() == crate::provider::ProviderKind::Apple {
        return crate::apple::get_container_mounts(&id);
    }
    let info = get_client(&docker)?
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await
        .map_err(|e| e.to_string())?;

    let mounts = info
        .mounts
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let source = m.source?;
            let mount_type = m
                .typ
                .map(|t| format!("{:?}", t).to_lowercase())
                .unwrap_or_default();

            // Only bind mounts have real host paths accessible from macOS.
            // Volume mounts (/var/lib/docker/volumes/...) are inside the Docker VM.
            // Also skip sockets and /proc, /sys, /dev paths.
            if mount_type != "bind" {
                return None;
            }
            if source.starts_with("/var/run")
                || source.starts_with("/proc")
                || source.starts_with("/sys")
                || source.starts_with("/dev")
            {
                return None;
            }

            Some(MountInfo {
                mount_type,
                source,
                destination: m.destination.unwrap_or_default(),
                mode: m.mode.unwrap_or_default(),
                rw: m.rw.unwrap_or(true),
            })
        })
        .collect();

    Ok(mounts)
}

// --- Open in Finder ---

#[tauri::command]
pub async fn open_in_finder(path: String) -> Result<(), String> {
    let output = Command::new("open")
        .args(["-R", &path])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Cannot open path: {}", err));
    }
    Ok(())
}

// --- Terminal ---

#[tauri::command]
pub async fn detect_terminal() -> Result<String, String> {
    let terminals = [
        ("/Applications/Ghostty.app", "ghostty"),
        ("/Applications/iTerm.app", "iterm"),
    ];

    for (path, name) in &terminals {
        if Path::new(path).exists() {
            return Ok(name.to_string());
        }
    }

    Ok("terminal".to_string())
}

#[tauri::command]
pub async fn open_terminal(
    provider: State<'_, crate::provider::ProviderState>,
    container_id: String,
    _container_name: String,
    shell: Option<String>,
    terminal_override: Option<String>,
) -> Result<(), String> {
    validate_container_id(&container_id)?;

    // Validate shell against allowlist
    let allowed_shells = ["/bin/sh", "/bin/bash", "/bin/zsh", "/bin/ash"];
    let sh = shell.unwrap_or_else(|| "/bin/sh".to_string());
    if !allowed_shells.contains(&sh.as_str()) {
        return Err(format!("Shell not allowed: {}", sh));
    }

    let terminal = match terminal_override {
        Some(ref t) if t != "auto" => t.clone(),
        _ => detect_terminal().await?,
    };
    let selected_provider = provider.get();
    let exec_cmd = if selected_provider == crate::provider::ProviderKind::Apple {
        format!("container exec -i -t {} {}", container_id, sh)
    } else {
        let host = crate::runtime::docker_host_for(selected_provider)
            .map(|value| format!("DOCKER_HOST={} ", value))
            .unwrap_or_default();
        let docker_bin = crate::runtime::docker_cli_for(selected_provider);
        format!(
            "{}\"{}\" exec -it {} {}",
            host,
            docker_bin.display(),
            container_id,
            sh
        )
    };

    // Write a temp script so the terminal runs a single clean command
    let tmp = std::env::temp_dir().join(format!("docker-tray-{}.sh", container_id));
    std::fs::write(&tmp, format!("#!/bin/sh\n{}\n", exec_cmd)).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }

    let tmp_str = tmp.to_string_lossy().to_string();
    let tmp_escaped = tmp_str.replace('\\', "\\\\").replace('"', "\\\"");

    match terminal.as_str() {
        "ghostty" => {
            Command::new("/Applications/Ghostty.app/Contents/MacOS/ghostty")
                .args(["-e", &tmp_str])
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        "iterm" => {
            let script = format!(
                r#"tell application "iTerm"
                    activate
                    create window with default profile command "{}"
                end tell"#,
                tmp_escaped
            );
            Command::new("osascript")
                .args(["-e", &script])
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        _ => {
            Command::new("open")
                .args(["-a", "Terminal", &tmp_str])
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    };

    Ok(())
}

// --- File Explorer ---

#[derive(Debug, Serialize, Clone)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: String,
    pub modified: String,
    pub permissions: String,
}

#[tauri::command]
pub async fn list_container_files(
    provider: State<'_, crate::provider::ProviderState>,
    container_id: String,
    path: String,
) -> Result<Vec<FileEntry>, String> {
    validate_container_id(&container_id)?;
    let p = provider.get();
    // Try GNU ls first, fallback to plain ls for BusyBox/Alpine
    let output = exec_cmd(p, "exec")
        .args([&container_id, "ls", "-la", "--time-style=long-iso", &path])
        .output()
        .map_err(|e| e.to_string())?;

    let (stdout, is_gnu) = if output.status.success() {
        (String::from_utf8_lossy(&output.stdout).to_string(), true)
    } else {
        // Fallback: plain ls -la (BusyBox)
        let fallback = exec_cmd(p, "exec")
            .args([&container_id, "ls", "-la", &path])
            .output()
            .map_err(|e| e.to_string())?;
        if !fallback.status.success() {
            let err = String::from_utf8_lossy(&fallback.stderr);
            return Err(format!("Failed to list files: {}", err));
        }
        (String::from_utf8_lossy(&fallback.stdout).to_string(), false)
    };

    let entries: Vec<FileEntry> = stdout
        .lines()
        .skip(1) // skip "total N" line
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // GNU: perms links owner group size date time name...
            // BusyBox: perms links owner group size mon day time/year name...
            if parts.len() < 8 {
                return None;
            }

            let (size_idx, name_start, modified) = if is_gnu {
                // GNU: parts[4]=size, parts[5]=date, parts[6]=time, parts[7..]=name
                (4, 7, format!("{} {}", parts[5], parts[6]))
            } else {
                // BusyBox: parts[4]=size, parts[5]=mon, parts[6]=day, parts[7]=time/year, parts[8..]=name
                if parts.len() < 9 {
                    // Some BusyBox outputs have fewer columns
                    (
                        4,
                        8.min(parts.len()),
                        format!(
                            "{} {}",
                            parts.get(5).unwrap_or(&""),
                            parts.get(6).unwrap_or(&"")
                        ),
                    )
                } else {
                    (4, 8, format!("{} {} {}", parts[5], parts[6], parts[7]))
                }
            };

            if name_start >= parts.len() {
                return None;
            }

            let name = parts[name_start..].join(" ");
            if name == "." || name == ".." {
                return None;
            }
            let display_name = if let Some(idx) = name.find(" -> ") {
                name[..idx].to_string()
            } else {
                name
            };
            Some(FileEntry {
                is_dir: parts[0].starts_with('d'),
                permissions: parts[0].to_string(),
                size: parts.get(size_idx).unwrap_or(&"").to_string(),
                modified,
                name: display_name,
            })
        })
        .collect();

    Ok(entries)
}

#[tauri::command]
pub async fn read_container_file(
    provider: State<'_, crate::provider::ProviderState>,
    container_id: String,
    path: String,
) -> Result<String, String> {
    validate_container_id(&container_id)?;
    let output = exec_cmd(provider.get(), "exec")
        .args([&container_id, "cat", &path])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to read file: {}", err));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub async fn save_from_container(
    provider: State<'_, crate::provider::ProviderState>,
    container_id: String,
    container_path: String,
    host_path: String,
) -> Result<(), String> {
    validate_container_id(&container_id)?;
    let src = format!("{}:{}", container_id, container_path);
    let output = exec_cmd(provider.get(), "cp")
        .args([&src, &host_path])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to copy: {}", err));
    }
    Ok(())
}

#[tauri::command]
pub async fn import_to_container(
    provider: State<'_, crate::provider::ProviderState>,
    container_id: String,
    host_path: String,
    container_path: String,
) -> Result<(), String> {
    validate_container_id(&container_id)?;
    let dest = format!("{}:{}", container_id, container_path);
    let output = exec_cmd(provider.get(), "cp")
        .args([&host_path, &dest])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to import: {}", err));
    }
    Ok(())
}
