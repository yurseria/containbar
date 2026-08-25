//! Apple Container CLI (`container`) adapter.
//!
//! Apple's native `container` tool does NOT expose a Docker Engine API socket,
//! so every operation shells out to the `container` binary and parses its
//! `--format json` / `inspect` JSON output. Because the exact JSON field names
//! are not documented and vary between releases, we deserialize into
//! `serde_json::Value` and probe a small set of candidate keys.

use crate::docker::{ContainerGroup, ContainerInfo, ImageInfo, NetworkInfo, PortInfo, VolumeInfo};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::process::{Command, Stdio};

/// Resolve the `container` binary. Allows overriding via CONTAINER_BIN for dev.
pub(crate) fn container_bin() -> String {
    if let Ok(path) = std::env::var("CONTAINER_BIN") {
        return path;
    }
    for path in ["/opt/homebrew/bin/container", "/usr/local/bin/container"] {
        if std::path::Path::new(path).is_file() {
            return path.to_string();
        }
    }
    "container".to_string()
}

/// Build a `container` command.
fn container_cmd() -> Command {
    Command::new(container_bin())
}

/// Run a `container` command and return (stdout, stderr, success).
fn run(cmd: &mut Command) -> Result<(String, String, bool), String> {
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run container CLI: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((stdout, stderr, output.status.success()))
}

fn last_error(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("Unknown container CLI error")
        .to_string()
}

/// Coerce a JSON value into a String, trimming a leading `sha256:` if present
/// when `digest` is set (for image IDs).
fn val_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => v.to_string(),
    }
}

/// Pick the first present key from a JSON object.
fn pick<'a>(obj: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    if let Value::Object(map) = obj {
        for k in keys {
            if let Some(v) = map.get(*k) {
                if !v.is_null() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn pick_str(obj: &Value, keys: &[&str]) -> String {
    pick(obj, keys).map(val_string).unwrap_or_default()
}

fn pointer_str(obj: &Value, pointers: &[&str]) -> String {
    pointers
        .iter()
        .find_map(|pointer| obj.pointer(pointer).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn string_map(value: Option<&Value>) -> HashMap<String, String> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect()
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// True if the `container` binary is installed and runnable.
pub fn apple_container_available() -> bool {
    Command::new(container_bin())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True when the Apple Container API service is healthy. Unlike `container
/// list`, this probe does not depend on any resource-list output format.
pub fn system_running() -> bool {
    Command::new(container_bin())
        .args(["system", "status", "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Ensure the Containerization framework backend is running. Idempotent.
/// `container system start` returns success if already running.
pub fn system_start() -> Result<(), String> {
    if system_running() {
        return Ok(());
    }

    let (_out, err, ok) = {
        let mut c = container_cmd();
        // The CLI prompts about installing its default Linux kernel unless
        // one of these flags is supplied. GUI processes have no interactive
        // stdin, so opt into the recommended kernel explicitly.
        c.args(["system", "start", "--enable-kernel-install"])
            .stdin(Stdio::null());
        run(&mut c)
    }?;
    if ok && system_running() {
        return Ok(());
    }
    Err(format!(
        "container system start failed: {}",
        last_error(&err)
    ))
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

/// Parse one container row from `container list --format json`.
/// The schema is undocumented, so we probe common key spellings.
fn container_from_json(obj: &Value) -> ContainerInfo {
    let id = {
        let top_level = pick_str(obj, &["id", "name", "containerID", "container_id"]);
        if top_level.is_empty() {
            pointer_str(obj, &["/configuration/id"])
        } else {
            top_level
        }
    };
    let image = {
        let reference = pointer_str(
            obj,
            &["/configuration/image/reference", "/configuration/imageRef"],
        );
        if reference.is_empty() {
            pick_str(obj, &["image", "imageRef", "image_ref"])
        } else {
            reference
        }
    };
    let state = {
        let nested = pointer_str(obj, &["/status/state"]);
        if nested.is_empty() {
            pick(obj, &["state", "status"])
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        } else {
            nested
        }
    };
    let status = if state.is_empty() {
        "unknown".to_string()
    } else {
        state.clone()
    };
    let created = obj
        .pointer("/configuration/creationDate")
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
        .or_else(|| {
            pick(obj, &["created", "createdAt", "created_at", "creation"]).and_then(Value::as_i64)
        })
        .unwrap_or(0);
    let ports = obj
        .pointer("/configuration/publishedPorts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|port| {
            Some(PortInfo {
                private_port: port.get("containerPort")?.as_u64()?.try_into().ok()?,
                public_port: port
                    .get("hostPort")
                    .and_then(Value::as_u64)
                    .and_then(|port| port.try_into().ok()),
                port_type: port
                    .get("proto")
                    .and_then(Value::as_str)
                    .unwrap_or("tcp")
                    .to_string(),
            })
        })
        .collect();
    let labels = string_map(
        obj.pointer("/configuration/labels")
            .or_else(|| obj.get("labels")),
    );

    ContainerInfo {
        // Apple uses human-readable names as IDs. Truncating them made every
        // service in a Compose project share the same React key and action ID.
        id: id.clone(),
        names: vec![id],
        image,
        state,
        status,
        ports,
        created,
        labels,
    }
}

fn labels_from_mocker_inspect(obj: &Value) -> HashMap<String, String> {
    string_map(
        obj.pointer("/Config/Labels")
            .or_else(|| obj.pointer("/config/labels"))
            .or_else(|| obj.get("labels")),
    )
}

fn mocker_container_labels() -> HashMap<String, HashMap<String, String>> {
    let Some(mocker) = crate::runtime::mocker_cli() else {
        return HashMap::new();
    };
    let Ok(ids) = Command::new(&mocker).args(["ps", "-a", "-q"]).output() else {
        return HashMap::new();
    };
    if !ids.status.success() {
        return HashMap::new();
    }
    let ids = String::from_utf8_lossy(&ids.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return HashMap::new();
    }

    let Ok(inspect) = Command::new(mocker).arg("inspect").args(&ids).output() else {
        return HashMap::new();
    };
    if !inspect.status.success() {
        return HashMap::new();
    }
    let Ok(items) = serde_json::from_slice::<Vec<Value>>(&inspect.stdout) else {
        return HashMap::new();
    };

    let mut by_identifier = HashMap::new();
    for item in items {
        let labels = labels_from_mocker_inspect(&item);
        if labels.is_empty() {
            continue;
        }
        for identifier in [
            pick_str(&item, &["Id", "ID", "id", "containerID", "container_id"]),
            pick_str(&item, &["Name", "name"])
                .trim_start_matches('/')
                .to_string(),
        ] {
            if !identifier.is_empty() {
                by_identifier.insert(identifier.clone(), labels.clone());
                by_identifier.insert(identifier.chars().take(12).collect(), labels.clone());
            }
        }
    }
    by_identifier
}

fn group_apple_containers(containers: Vec<ContainerInfo>) -> Vec<ContainerGroup> {
    let mut groups = BTreeMap::<String, Vec<ContainerInfo>>::new();
    let mut standalone = Vec::new();
    for container in containers {
        let project = container
            .labels
            .get("com.mocker.compose.project")
            .or_else(|| container.labels.get("com.docker.compose.project"))
            .cloned();
        if let Some(project) = project {
            groups.entry(project).or_default().push(container);
        } else {
            standalone.push(container);
        }
    }

    let mut result = groups
        .into_iter()
        .map(|(name, containers)| ContainerGroup { name, containers })
        .collect::<Vec<_>>();
    if !standalone.is_empty() {
        result.push(ContainerGroup {
            name: "Standalone".to_string(),
            containers: standalone,
        });
    }
    result
}

pub fn list_containers() -> Result<Vec<ContainerGroup>, String> {
    let (stdout, stderr, ok) = {
        let mut c = container_cmd();
        c.args(["list", "--all", "--format", "json"]);
        run(&mut c)
    }?;
    if !ok {
        return Err(last_error(&stderr));
    }

    let trimmed = stdout.trim();
    let items: Vec<Value> = if trimmed.is_empty() {
        Vec::new()
    } else if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).map_err(|e| format!("Invalid list JSON: {e}"))?
    } else {
        // Some versions emit JSON-lines; tolerate that.
        trimmed
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str::<Value>)
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Invalid list JSON line: {e}"))?
    };

    let mut containers = items.iter().map(container_from_json).collect::<Vec<_>>();

    // Current Apple output includes Mocker labels under configuration.labels.
    // Keep the inspect fallback for older CLI versions that omit them.
    if containers
        .iter()
        .any(|container| container.labels.is_empty())
    {
        let labels = mocker_container_labels();
        for container in &mut containers {
            let metadata = container
                .names
                .iter()
                .chain(std::iter::once(&container.id))
                .find_map(|identifier| labels.get(identifier));
            if let Some(metadata) = metadata {
                container.labels = metadata.clone();
            }
        }
    }

    Ok(group_apple_containers(containers))
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

fn normalize_image_reference(reference: &str) -> String {
    reference
        .strip_prefix("docker.io/library/")
        .or_else(|| reference.strip_prefix("docker.io/"))
        .unwrap_or(reference)
        .to_string()
}

fn image_variant_for_arch<'a>(obj: &'a Value, arch: &str) -> Option<&'a Value> {
    let variants = obj.pointer("/variants")?.as_array()?;
    variants
        .iter()
        .find(|variant| {
            variant.pointer("/platform/os").and_then(Value::as_str) == Some("linux")
                && variant
                    .pointer("/platform/architecture")
                    .and_then(Value::as_str)
                    == Some(arch)
        })
        .or_else(|| {
            variants
                .iter()
                .find(|variant| variant.get("size").is_some())
        })
}

fn apple_image_from_json_for_arch(obj: &Value, arch: &str) -> ImageInfo {
    let variant = image_variant_for_arch(obj, arch);
    let id = pointer_str(obj, &["/configuration/descriptor/digest"]);
    let id = if id.is_empty() {
        pick_str(obj, &["id", "digest", "imageID", "image_id"])
    } else {
        id
    };

    let mut repo_tags = Vec::new();
    let nested_name = pointer_str(obj, &["/configuration/name"]);
    if !nested_name.is_empty() {
        repo_tags.push(normalize_image_reference(&nested_name));
    } else if let Some(value) = pick(obj, &["name", "reference", "repoTags", "repo_tags"]) {
        match value {
            Value::String(tag) => repo_tags.push(normalize_image_reference(tag)),
            Value::Array(tags) => repo_tags.extend(
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(normalize_image_reference),
            ),
            _ => {}
        }
    }

    let created = obj
        .pointer("/configuration/creationDate")
        .and_then(Value::as_str)
        .or_else(|| variant.and_then(|variant| variant.pointer("/config/created")?.as_str()))
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
        .or_else(|| {
            pick(obj, &["created", "createdAt", "created_at", "creation"]).and_then(Value::as_i64)
        })
        .unwrap_or(0);

    ImageInfo {
        id: id.trim_start_matches("sha256:").chars().take(12).collect(),
        repo_tags,
        // An image index contains every platform. Show only the host platform's
        // image size, as Docker does, instead of adding all variants together.
        size: variant
            .and_then(|variant| variant.get("size"))
            .and_then(Value::as_i64)
            .or_else(|| {
                obj.pointer("/configuration/descriptor/size")
                    .and_then(Value::as_i64)
            })
            .or_else(|| pick(obj, &["size", "fullSize", "full_size"]).and_then(Value::as_i64))
            .unwrap_or(0),
        created,
    }
}

fn apple_image_from_json(obj: &Value) -> ImageInfo {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        arch => arch,
    };
    apple_image_from_json_for_arch(obj, arch)
}

fn parse_mocker_size(value: &str) -> Option<i64> {
    let mut parts = value.split_whitespace();
    let amount = parts.next()?.parse::<f64>().ok()?;
    let multiplier = match parts.next()?.to_ascii_uppercase().as_str() {
        "B" => 1_f64,
        "KB" => 1_000_f64,
        "KIB" => 1_024_f64,
        "MB" => 1_000_000_f64,
        "MIB" => 1_048_576_f64,
        "GB" => 1_000_000_000_f64,
        "GIB" => 1_073_741_824_f64,
        "TB" => 1_000_000_000_000_f64,
        "TIB" => 1_099_511_627_776_f64,
        _ => return None,
    };
    Some((amount * multiplier).round() as i64)
}

fn list_mocker_images(apple_images: &[ImageInfo]) -> Option<Vec<ImageInfo>> {
    let mocker = crate::runtime::mocker_cli()?;
    let output = Command::new(mocker)
        .args([
            "images",
            "--no-trunc",
            "--format",
            "{{.Repository}}\t{{.Tag}}\t{{.ID}}\t{{.Size}}",
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let metadata = apple_images
        .iter()
        .map(|image| (image.id.as_str(), image))
        .collect::<HashMap<_, _>>();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut images = Vec::<ImageInfo>::new();
    let mut indexes = HashMap::<String, usize>::new();

    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let columns = line.split('\t').collect::<Vec<_>>();
        if columns.len() != 4 {
            continue;
        }
        let id = columns[2]
            .trim_start_matches("sha256:")
            .chars()
            .take(12)
            .collect::<String>();
        if id.is_empty() {
            continue;
        }
        let tag = if columns[0] == "<none>" || columns[1] == "<none>" {
            None
        } else {
            Some(normalize_image_reference(&format!(
                "{}:{}",
                columns[0], columns[1]
            )))
        };

        if let Some(index) = indexes.get(&id).copied() {
            if let Some(tag) = tag {
                if !images[index].repo_tags.contains(&tag) {
                    images[index].repo_tags.push(tag);
                }
            }
            continue;
        }

        let apple = metadata.get(id.as_str()).copied();
        let image = ImageInfo {
            id: id.clone(),
            repo_tags: tag.into_iter().collect(),
            size: apple
                .filter(|image| image.size > 0)
                .map(|image| image.size)
                .or_else(|| parse_mocker_size(columns[3]))
                .unwrap_or(0),
            created: apple.map(|image| image.created).unwrap_or(0),
        };
        indexes.insert(id, images.len());
        images.push(image);
    }

    (!images.is_empty() || apple_images.is_empty()).then_some(images)
}

pub fn list_images() -> Result<Vec<ImageInfo>, String> {
    let (stdout, stderr, ok) = {
        let mut c = container_cmd();
        c.args(["image", "list", "--format", "json"]);
        run(&mut c)
    }?;
    if !ok {
        return Err(last_error(&stderr));
    }

    let trimmed = stdout.trim();
    let items: Vec<Value> = if trimmed.is_empty() {
        Vec::new()
    } else if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).map_err(|e| format!("Invalid image list JSON: {e}"))?
    } else {
        trimmed
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str::<Value>)
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Invalid image list JSON line: {e}"))?
    };

    let apple_images = items.iter().map(apple_image_from_json).collect::<Vec<_>>();
    Ok(list_mocker_images(&apple_images).unwrap_or(apple_images))
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

pub fn start_container(id: &str) -> Result<(), String> {
    let (_out, err, ok) = {
        let mut c = container_cmd();
        c.args(["start", id]);
        run(&mut c)
    }?;
    if ok {
        Ok(())
    } else {
        Err(last_error(&err))
    }
}

pub fn stop_container(id: &str) -> Result<(), String> {
    let (_out, err, ok) = {
        let mut c = container_cmd();
        c.args(["stop", id]);
        run(&mut c)
    }?;
    if ok {
        Ok(())
    } else {
        Err(last_error(&err))
    }
}

/// Apple Container has no `restart`; emulate with stop then start.
pub fn restart_container(id: &str) -> Result<(), String> {
    // Best-effort stop: ignore "not running" style failures, then start.
    let _ = container_cmd().args(["stop", id]).output();
    start_container(id)
}

pub fn remove_container(id: &str, force: bool) -> Result<(), String> {
    let mut args = vec!["delete"];
    if force {
        args.push("--force");
    }
    args.push(id);
    let (_out, err, ok) = {
        let mut c = container_cmd();
        c.args(&args);
        run(&mut c)
    }?;
    if ok {
        Ok(())
    } else {
        Err(last_error(&err))
    }
}

pub fn remove_image(image: &str) -> Result<(), String> {
    let (_out, err, ok) = {
        let mut c = container_cmd();
        c.args(["image", "delete", "--force", image]);
        run(&mut c)
    }?;
    if ok {
        Ok(())
    } else {
        Err(last_error(&err))
    }
}

pub fn pull_image(image: &str) -> Result<(), String> {
    // --progress none avoids TTY control sequences in captured stdout.
    let (_out, err, ok) = {
        let mut c = container_cmd();
        c.args(["image", "pull", "--progress", "none", image]);
        run(&mut c)
    }?;
    if ok {
        Ok(())
    } else {
        Err(last_error(&err))
    }
}

// ---------------------------------------------------------------------------
// Create / Run
// ---------------------------------------------------------------------------

pub fn create_container(input: &crate::docker::CreateContainerInput) -> Result<String, String> {
    let mut args: Vec<String> = vec!["run".to_string()];

    if let Some(name) = &input.name {
        args.push("--name".to_string());
        args.push(name.clone());
    }

    for p in &input.ports {
        // Apple format: [host-ip:]host-port:container-port[/protocol]
        args.push("-p".to_string());
        args.push(format!("{}:{}", p.host, p.container));
    }

    for v in &input.volumes {
        args.push("-v".to_string());
        args.push(format!("{}:{}", v.host, v.container));
    }

    for e in &input.env {
        args.push("-e".to_string());
        args.push(e.clone());
    }

    if input.auto_start {
        args.push("-d".to_string());
    } else {
        args.push("create".to_string());
        // Replace the leading "run" with "create".
        args[0] = "create".to_string();
    }

    args.push(input.image.clone());

    let (stdout, stderr, ok) = {
        let mut c = container_cmd();
        c.args(&args);
        run(&mut c)
    }?;
    if !ok {
        return Err(last_error(&stderr));
    }
    // The new container ID is printed on stdout (detached) — use the name if
    // the caller supplied one (Apple treats name == ID), else the printed ID.
    let printed = stdout.trim().to_string();
    Ok(if let Some(name) = &input.name {
        if name.is_empty() {
            printed
        } else {
            name.clone()
        }
    } else {
        printed
    })
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

pub fn get_container_logs(id: &str, tail: Option<&str>) -> Result<Vec<String>, String> {
    let mut args = vec!["logs".to_string()];
    if let Some(n) = tail {
        if n != "all" {
            args.push("-n".to_string());
            args.push(n.to_string());
        }
    }
    args.push(id.to_string());

    let (stdout, stderr, ok) = {
        let mut c = container_cmd();
        c.args(&args);
        run(&mut c)
    }?;
    if !ok {
        return Err(last_error(&stderr));
    }

    // Apple `container logs` does not split stdout/stderr per line markers like
    // Docker; combine both streams, preserving order.
    let mut lines: Vec<String> = stdout
        .lines()
        .map(String::from)
        .chain(stderr.lines().map(String::from))
        .collect();
    if lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    Ok(lines)
}

/// Apple Container has no `--since` flag. As a fallback we fetch the last
/// `tail` lines; the frontend treats this response as a rolling snapshot and
/// appends only the non-overlapping suffix.
pub fn get_container_logs_since(
    id: &str,
    _since: i64,
    _timestamps: bool,
) -> Result<Vec<String>, String> {
    get_container_logs(id, Some("200"))
}

// ---------------------------------------------------------------------------
// Inspect — env & mounts
// ---------------------------------------------------------------------------

/// Run `container inspect <id>` and return the parsed JSON object.
fn inspect(id: &str) -> Result<Value, String> {
    let (stdout, stderr, ok) = {
        let mut c = container_cmd();
        c.args(["inspect", id]);
        run(&mut c)
    }?;
    if !ok {
        return Err(last_error(&stderr));
    }
    let trimmed = stdout.trim();
    // inspect may emit a single object or an array of objects.
    if trimmed.starts_with('[') {
        let arr: Vec<Value> =
            serde_json::from_str(trimmed).map_err(|e| format!("Invalid inspect JSON: {e}"))?;
        arr.into_iter()
            .next()
            .ok_or_else(|| "Empty inspect result".to_string())
    } else {
        serde_json::from_str(trimmed).map_err(|e| format!("Invalid inspect JSON: {e}"))
    }
}

pub fn get_container_env(id: &str) -> Result<Vec<String>, String> {
    let obj = inspect(id)?;

    // Probe nested config locations where env typically lives.
    let candidates: Vec<&str> = vec!["config", "process", "container", "specification", "spec"];
    let mut env: Vec<String> = Vec::new();

    let mut scan = |o: &Value| {
        for key in &candidates {
            if let Some(sub) = o.get(*key) {
                if let Some(e) = find_env_in(sub) {
                    env = e;
                    return;
                }
            }
        }
        if let Some(e) = find_env_in(o) {
            env = e;
        }
    };
    scan(&obj);

    Ok(env)
}

/// Search a JSON subtree for an `env` array of strings.
fn find_env_in(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if k == "env" || k == "environment" {
                    if let Value::Array(arr) = val {
                        let out: Vec<String> = arr
                            .iter()
                            .map(|e| match e {
                                Value::String(s) => s.clone(),
                                _ => val_string(e),
                            })
                            .collect();
                        if !out.is_empty() {
                            return Some(out);
                        }
                    }
                }
            }
            for (_, val) in map {
                if let Some(found) = find_env_in(val) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

pub fn get_container_mounts(id: &str) -> Result<Vec<crate::docker::MountInfo>, String> {
    let obj = inspect(id)?;

    let mut mounts: Vec<crate::docker::MountInfo> = Vec::new();
    collect_mounts(&obj, &mut mounts);

    // Mirror the Docker adapter's filtering: only bind mounts with a real,
    // accessible host path; skip sockets and /proc /sys /dev.
    Ok(mounts
        .into_iter()
        .filter(|m| {
            m.mount_type == "bind"
                && !m.source.starts_with("/var/run")
                && !m.source.starts_with("/proc")
                && !m.source.starts_with("/sys")
                && !m.source.starts_with("/dev")
        })
        .collect())
}

fn collect_mounts(v: &Value, out: &mut Vec<crate::docker::MountInfo>) {
    match v {
        Value::Object(map) => {
            // A mounts array typically lives under "mounts" / "volumes".
            for key in &["mounts", "volumes", "mount"] {
                if let Some(Value::Array(arr)) = map.get(*key) {
                    for item in arr {
                        if let Some(m) = parse_mount(item) {
                            out.push(m);
                        }
                    }
                }
            }
            for (_, val) in map {
                collect_mounts(val, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_mounts(item, out);
            }
        }
        _ => {}
    }
}

fn parse_mount(item: &Value) -> Option<crate::docker::MountInfo> {
    let source = pick_str(item, &["source", "hostPath", "host_path", "src"]);
    let destination = pick_str(
        item,
        &["destination", "target", "containerPath", "container_path"],
    );
    if source.is_empty() && destination.is_empty() {
        return None;
    }
    let mount_type = pick_str(item, &["type", "mountType", "mount_type"]);
    let mount_type = if mount_type.is_empty() {
        "bind".to_string()
    } else {
        mount_type.to_lowercase()
    };
    let rw = pick(item, &["rw", "readWrite", "read_write", "writable"])
        .map(|v| match v {
            Value::Bool(b) => *b,
            Value::String(s) => s == "true" || s == "rw",
            _ => true,
        })
        .unwrap_or(true);
    let mode = pick_str(item, &["mode", "options", "propagation"]);

    Some(crate::docker::MountInfo {
        mount_type,
        source,
        destination,
        mode,
        rw,
    })
}

// ---------------------------------------------------------------------------
// Volumes & Networks (best-effort; networks require macOS 26+)
// ---------------------------------------------------------------------------

pub fn list_volumes() -> Result<Vec<VolumeInfo>, String> {
    // Apple Compose is driven by Mocker, whose named volumes are host-backed
    // directories under ~/.mocker. Showing native `container volume` entries
    // here would hide the volumes actually used by Compose projects.
    let Some(mocker) = crate::runtime::mocker_cli() else {
        return Ok(Vec::new());
    };
    let names = Command::new(&mocker)
        .args(["volume", "ls", "-q"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Failed to list Mocker volumes: {error}"))?;
    if !names.status.success() {
        return Err(last_error(&String::from_utf8_lossy(&names.stderr)));
    }

    let mut volumes = Vec::new();
    for name in String::from_utf8_lossy(&names.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let inspect = Command::new(&mocker)
            .args(["volume", "inspect", name])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("Failed to inspect Mocker volume {name}: {error}"))?;
        if !inspect.status.success() {
            return Err(last_error(&String::from_utf8_lossy(&inspect.stderr)));
        }
        let value = serde_json::from_slice::<Value>(&inspect.stdout)
            .map_err(|error| format!("Invalid Mocker volume response for {name}: {error}"))?;
        let item = value
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or(&value);
        volumes.push(mocker_volume_from_json(item, name));
    }
    volumes.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(volumes)
}

fn mocker_volume_from_json(obj: &Value, fallback_name: &str) -> VolumeInfo {
    let name = pick_str(obj, &["name"]);
    VolumeInfo {
        name: if name.is_empty() {
            fallback_name.to_string()
        } else {
            name
        },
        driver: {
            let driver = pick_str(obj, &["driver"]);
            if driver.is_empty() {
                "local".to_string()
            } else {
                driver.to_lowercase()
            }
        },
        mountpoint: pick_str(obj, &["mountpoint", "mountPoint", "mount_point", "path"]),
        labels: string_map(obj.get("labels")),
    }
}

pub fn list_networks() -> Result<Vec<NetworkInfo>, String> {
    let (stdout, stderr, ok) = {
        let mut c = container_cmd();
        c.args(["network", "list", "--format", "json"]);
        run(&mut c)
    }?;
    if !ok {
        // Networks are macOS 26+ only; tolerate older versions gracefully.
        let msg = last_error(&stderr);
        if msg.to_lowercase().contains("unknown command") || msg.to_lowercase().contains("no such")
        {
            return Ok(Vec::new());
        }
        return Err(msg);
    }

    let trimmed = stdout.trim();
    let items: Vec<Value> = if trimmed.is_empty() {
        Vec::new()
    } else if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).map_err(|e| format!("Invalid network list JSON: {e}"))?
    } else {
        trimmed
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str::<Value>)
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Invalid network list JSON line: {e}"))?
    };

    let container_counts = apple_network_container_counts();
    Ok(items
        .iter()
        .map(|obj| apple_network_from_json(obj, &container_counts))
        .collect())
}

fn apple_network_container_counts() -> HashMap<String, usize> {
    let Ok(output) = Command::new(container_bin())
        .args(["list", "--all", "--format", "json"])
        .stdin(Stdio::null())
        .output()
    else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }
    let Ok(items) = serde_json::from_slice::<Vec<Value>>(&output.stdout) else {
        return HashMap::new();
    };
    let mut counts = HashMap::new();
    for item in items {
        let Some(networks) = item.pointer("/status/networks").and_then(Value::as_array) else {
            continue;
        };
        for network in networks {
            let Some(name) = network.pointer("/network").and_then(Value::as_str) else {
                continue;
            };
            *counts.entry(name.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

fn apple_network_from_json(obj: &Value, counts: &HashMap<String, usize>) -> NetworkInfo {
    let id = pick_str(obj, &["id"]);
    let name = pointer_str(obj, &["/configuration/name"]);
    let driver = pointer_str(obj, &["/configuration/plugin"]);
    let mode = pointer_str(obj, &["/configuration/mode"]);
    NetworkInfo {
        // Apple uses the human-readable name as the real resource ID. Docker's
        // 12-character hash truncation must not be applied here.
        id: if id.is_empty() { name.clone() } else { id },
        name: if name.is_empty() {
            pick_str(obj, &["id"])
        } else {
            name.clone()
        },
        driver: if driver.is_empty() {
            "apple".to_string()
        } else {
            driver
        },
        // NetworkInfo is shared with Docker, which calls this field `scope`.
        // Apple's closest useful equivalent is its network mode (`nat`, etc.).
        scope: mode,
        containers: counts.get(&name).copied().unwrap_or(0),
    }
}

pub fn remove_volume(name: &str) -> Result<(), String> {
    let mocker = crate::runtime::mocker_cli()
        .ok_or_else(|| "Mocker is not installed; no Mocker volumes can be removed".to_string())?;
    let output = Command::new(mocker)
        .args(["volume", "rm", name])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Failed to remove Mocker volume {name}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(last_error(&String::from_utf8_lossy(&output.stderr)))
    }
}

pub fn remove_network(name: &str) -> Result<(), String> {
    let (_out, err, ok) = {
        let mut c = container_cmd();
        c.args(["network", "delete", name]);
        run(&mut c)
    }?;
    if ok {
        Ok(())
    } else {
        Err(last_error(&err))
    }
}

#[cfg(test)]
mod container_group_tests {
    use super::{
        apple_image_from_json_for_arch, apple_network_from_json, container_from_json,
        group_apple_containers, labels_from_mocker_inspect, mocker_volume_from_json,
    };
    use crate::docker::ContainerInfo;
    use serde_json::json;
    use std::collections::HashMap;

    fn container(name: &str, labels: HashMap<String, String>) -> ContainerInfo {
        ContainerInfo {
            id: name.to_string(),
            names: vec![name.to_string()],
            image: "test:latest".to_string(),
            state: "running".to_string(),
            status: "running".to_string(),
            ports: Vec::new(),
            created: 0,
            labels,
        }
    }

    #[test]
    fn reads_and_groups_mocker_compose_labels() {
        let inspect = json!({
            "Config": {
                "Labels": {
                    "com.mocker.compose.project": "demo",
                    "com.mocker.compose.service": "web"
                }
            }
        });
        let labels = labels_from_mocker_inspect(&inspect);
        let groups = group_apple_containers(vec![
            container("demo-web-1", labels),
            container("manual", HashMap::new()),
        ]);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "demo");
        assert_eq!(groups[0].containers.len(), 1);
        assert_eq!(groups[1].name, "Standalone");
    }

    #[test]
    fn parses_nested_apple_container_list_fields() {
        let item = json!({
            "id": "demo-web-1",
            "configuration": {
                "creationDate": "2026-08-24T05:27:28Z",
                "image": { "reference": "docker.io/library/nginx:latest" },
                "labels": {
                    "com.mocker.compose.project": "demo",
                    "com.mocker.compose.service": "web"
                },
                "publishedPorts": [{
                    "containerPort": 80,
                    "hostPort": 8080,
                    "proto": "tcp"
                }]
            },
            "status": { "state": "running", "networks": [] }
        });

        let container = container_from_json(&item);
        assert_eq!(container.id, "demo-web-1");
        assert_eq!(container.image, "docker.io/library/nginx:latest");
        assert_eq!(container.state, "running");
        assert_eq!(container.status, "running");
        assert!(container.created > 0);
        assert_eq!(container.ports[0].public_port, Some(8080));
        assert_eq!(container.ports[0].private_port, 80);
        assert_eq!(
            container.labels.get("com.mocker.compose.project"),
            Some(&"demo".to_string())
        );
    }

    #[test]
    fn parses_docker_style_image_details_from_nested_apple_json() {
        let item = json!({
            "id": "becdda6c7f4b3fb42e42fd7f120bbf5c54c4caaaf16f26da24e4563d2c1f0576",
            "configuration": {
                "creationDate": "2026-08-19T08:40:35Z",
                "descriptor": {
                    "digest": "sha256:becdda6c7f4b3fb42e42fd7f120bbf5c54c4caaaf16f26da24e4563d2c1f0576",
                    "size": 1609
                },
                "name": "docker.io/library/redis:8-alpine"
            },
            "variants": [
                {
                    "platform": { "architecture": "amd64", "os": "linux" },
                    "size": 42_000_000
                },
                {
                    "config": { "created": "2026-08-19T08:41:00Z" },
                    "platform": { "architecture": "arm64", "os": "linux" },
                    "size": 38_700_000
                }
            ]
        });

        let image = apple_image_from_json_for_arch(&item, "arm64");
        assert_eq!(image.id, "becdda6c7f4b");
        assert_eq!(image.repo_tags, ["redis:8-alpine"]);
        assert_eq!(image.size, 38_700_000);
        assert!(image.created > 0);
    }

    #[test]
    fn parses_mocker_volume_details() {
        let item = json!({
            "driver": "local",
            "labels": { "com.example.owner": "demo" },
            "mountpoint": "/Users/demo/.mocker/volumes/demo-data/_data",
            "name": "demo-data"
        });

        let volume = mocker_volume_from_json(&item, "fallback");
        assert_eq!(volume.name, "demo-data");
        assert_eq!(volume.driver, "local");
        assert_eq!(
            volume.mountpoint,
            "/Users/demo/.mocker/volumes/demo-data/_data"
        );
        assert_eq!(
            volume.labels.get("com.example.owner"),
            Some(&"demo".to_string())
        );
    }

    #[test]
    fn preserves_full_apple_network_id_and_nested_fields() {
        let item = json!({
            "configuration": {
                "mode": "nat",
                "name": "demo-local-machine-network",
                "plugin": "container-network-vmnet"
            },
            "id": "demo-local-machine-network"
        });
        let counts = [("demo-local-machine-network".to_string(), 4)]
            .into_iter()
            .collect();

        let network = apple_network_from_json(&item, &counts);
        assert_eq!(network.id, "demo-local-machine-network");
        assert_eq!(network.name, "demo-local-machine-network");
        assert_eq!(network.driver, "container-network-vmnet");
        assert_eq!(network.scope, "nat");
        assert_eq!(network.containers, 4);
    }
}
