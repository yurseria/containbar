use crate::provider::ProviderKind;
use bollard::Docker;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum RuntimeKind {
    External, // Docker socket already available (Docker Desktop, OrbStack, etc.)
    Builtin,  // Bundled Colima
    Apple,    // Apple's native `container` CLI
    None,     // No runtime detected
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    pub kind: RuntimeKind,
    pub running: bool,
    pub message: String,
    pub provider: ProviderKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub provider: ProviderKind,
    pub installed: bool,
    pub running: bool,
    pub compatible: bool,
    pub detail: String,
}

/// Check if a non-Colima Docker socket is available (Docker Desktop, OrbStack, etc.)
pub fn external_docker_available() -> bool {
    external_docker_socket().is_some()
}

fn docker_socket_responds(socket: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;

        let Ok(mut stream) = UnixStream::connect(socket) else {
            return false;
        };
        let timeout = Some(Duration::from_secs(2));
        let _ = stream.set_read_timeout(timeout);
        let _ = stream.set_write_timeout(timeout);
        if stream
            .write_all(b"GET /_ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .is_err()
        {
            return false;
        }
        let mut response = String::new();
        stream.read_to_string(&mut response).is_ok()
            && response
                .lines()
                .next()
                .is_some_and(|line| line.contains(" 200 "))
            && response
                .split_once("\r\n\r\n")
                .is_some_and(|(_, body)| body.trim() == "OK")
    }

    #[cfg(not(unix))]
    {
        let _ = socket;
        false
    }
}

fn docker_cli() -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let candidates = [
        PathBuf::from("/opt/homebrew/bin/docker"),
        PathBuf::from("/usr/local/bin/docker"),
        PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker"),
        home.join(".orbstack/bin/docker"),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

pub fn docker_cli_for(provider: ProviderKind) -> PathBuf {
    if provider == ProviderKind::Colima {
        if let Some(path) = bundled_colima(&PathBuf::new())
            .and_then(|colima| runtime_base_from_colima(&colima))
            .map(|base| base.join("docker/bin/docker"))
            .filter(|path| path.is_file())
        {
            return path;
        }
    }
    docker_cli().unwrap_or_else(|| PathBuf::from("docker"))
}

fn docker_context_socket() -> Option<PathBuf> {
    let docker = docker_cli()?;
    let output = Command::new(docker)
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let host = String::from_utf8_lossy(&output.stdout).trim().to_string();
    host.strip_prefix("unix://").map(PathBuf::from)
}

fn configured_docker_socket() -> Option<PathBuf> {
    std::env::var("DOCKER_HOST")
        .ok()
        .and_then(|host| host.strip_prefix("unix://").map(PathBuf::from))
        .or_else(docker_context_socket)
}

fn same_socket(left: &Path, right: &Path) -> bool {
    left == right
        || left
            .canonicalize()
            .ok()
            .zip(right.canonicalize().ok())
            .is_some_and(|(left, right)| left == right)
}

/// Resolve the exact external Docker endpoint. The active Docker context is
/// considered first, followed by well-known sockets. Colima is deliberately
/// excluded so choosing Docker can never silently target the Colima VM.
pub fn external_docker_socket() -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let colima = colima_socket_path();
    let mut candidates = Vec::new();

    if let Some(path) = configured_docker_socket() {
        candidates.push(path);
    }
    candidates.extend([
        PathBuf::from("/var/run/docker.sock"),
        home.join(".orbstack/run/docker.sock"),
        home.join(".docker/run/docker.sock"),
    ]);

    let mut seen = HashSet::new();
    candidates.into_iter().find(|socket| {
        !same_socket(socket, &colima)
            && seen.insert(socket.clone())
            && socket.exists()
            && docker_socket_responds(socket)
    })
}

/// Suggest the runtime that best matches the machine's current environment.
/// The active Docker context wins; otherwise prefer an already-running engine
/// and finally Colima as the guided first-install default.
pub fn suggested_provider(resource_dir: &Path) -> ProviderKind {
    let colima = colima_socket_path();
    if let Some(configured) =
        configured_docker_socket().filter(|path| path.exists() && docker_socket_responds(path))
    {
        return if same_socket(&configured, &colima) {
            ProviderKind::Colima
        } else {
            ProviderKind::Docker
        };
    }

    if colima.exists() && docker_socket_responds(&colima) {
        return ProviderKind::Colima;
    }
    if external_docker_available() {
        return ProviderKind::Docker;
    }
    if provider_status(resource_dir, ProviderKind::Apple).running {
        return ProviderKind::Apple;
    }
    ProviderKind::Colima
}

/// Get the path to Colima binary — bundled first, then detect from running process
fn bundled_colima(resource_dir: &Path) -> Option<PathBuf> {
    // 1. Check bundled binary in resource dir
    let colima = resource_dir.join("runtime/colima/bin/colima");
    if colima.exists() {
        return Some(colima);
    }

    // 2. Detect from running colima process (for dev mode / installed app mismatch)
    //    macOS pgrep -a doesn't show command, so use ps instead
    if let Ok(output) = Command::new("ps").args(["-eo", "command"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("colima daemon") {
                if let Some(path) = line.split_whitespace().next() {
                    let p = PathBuf::from(path);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }

    // 3. Check installed app location (for dev mode where resource_dir differs)
    for path in [
        "/Applications/Containbar.app/Contents/Resources/runtime/colima/bin/colima",
        "/Applications/Docker Tray.app/Contents/Resources/runtime/colima/bin/colima",
    ] {
        let installed = PathBuf::from(path);
        if installed.exists() {
            return Some(installed);
        }
    }

    // 4. Check common system paths
    for path in &["/opt/homebrew/bin/colima", "/usr/local/bin/colima"] {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// Locate Homebrew even when the app was launched from Finder and inherited a
/// minimal PATH.
pub(crate) fn homebrew() -> Option<PathBuf> {
    for path in &["/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    Command::new("sh")
        .args(["-lc", "command -v brew"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!path.is_empty()).then(|| PathBuf::from(path))
        })
}

/// Resolve Mocker even when Containbar was launched from Finder with a
/// minimal PATH. `MOCKER_BIN` is useful for development and integration tests.
pub(crate) fn mocker_cli() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MOCKER_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    for path in ["/opt/homebrew/bin/mocker", "/usr/local/bin/mocker"] {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    Command::new("sh")
        .args(["-lc", "command -v mocker"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!path.is_empty()).then(|| PathBuf::from(path))
        })
        .filter(|path| path.is_file())
}

/// Ensure the Docker-compatible Compose adapter for Apple Container exists.
/// Mocker is installed lazily because Docker and Colima do not need it.
pub(crate) fn ensure_mocker() -> Result<PathBuf, String> {
    if let Some(path) = mocker_cli() {
        return Ok(path);
    }

    let brew = homebrew().ok_or(
        "Mocker is required for Compose on Apple Container, but Homebrew was not found. Install Homebrew, then try again.",
    )?;

    for (action, args) in [
        ("Adding the Mocker Homebrew tap", ["tap", "us/tap"]),
        ("Installing Mocker", ["install", "us/tap/mocker"]),
    ] {
        let output = Command::new(&brew)
            .args(args)
            .env("HOMEBREW_NO_ENV_HINTS", "1")
            .output()
            .map_err(|error| format!("Could not run Homebrew: {error}"))?;
        if !output.status.success() {
            return Err(command_error(action, &output));
        }
    }

    if let Some(path) = mocker_cli() {
        return Ok(path);
    }

    let prefix = Command::new(&brew)
        .args(["--prefix", "mocker"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|prefix| !prefix.is_empty())
        .map(PathBuf::from)
        .map(|prefix| prefix.join("bin/mocker"))
        .filter(|path| path.is_file());

    prefix.ok_or_else(|| {
        "Mocker was installed, but its executable could not be found. Restart the app and try again."
            .to_string()
    })
}

fn supported_apple_container_host() -> bool {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return false;
    }

    Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
        })
        .is_some_and(|major| major >= 26)
}

fn command_error(action: &str, output: &Output) -> String {
    let details = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let details = details.trim();

    if details.is_empty() {
        format!("{action} failed with status {}", output.status)
    } else {
        format!("{action} failed: {details}")
    }
}

/// Install a usable local runtime when a development or slim app bundle does
/// not contain Colima. This is also used by the Start Runtime button.
fn install_colima() -> Result<PathBuf, String> {
    let brew = homebrew().ok_or(
        "Colima is not bundled and Homebrew was not found. Install Homebrew, then try again.",
    )?;

    let mut formulas = Vec::new();
    for formula in ["colima", "docker"] {
        let installed = Command::new(&brew)
            .args(["list", "--formula", formula])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !installed {
            formulas.push(formula);
        }
    }

    if !formulas.is_empty() {
        let output = Command::new(&brew)
            .arg("install")
            .args(&formulas)
            .output()
            .map_err(|error| format!("Could not run Homebrew: {error}"))?;
        if !output.status.success() {
            return Err(command_error("Installing Colima", &output));
        }
    }

    bundled_colima(&PathBuf::new()).ok_or_else(|| {
        "Colima was installed, but its executable could not be found. Restart the app and try again."
            .to_string()
    })
}

fn ensure_colima(resource_dir: &Path) -> Result<PathBuf, String> {
    bundled_colima(resource_dir).map_or_else(install_colima, Ok)
}

pub fn ensure_apple_container() -> Result<(), String> {
    if !supported_apple_container_host() {
        return Err("Apple Container requires Apple Silicon and macOS 26 or later.".to_string());
    }
    if crate::apple::apple_container_available() {
        return Ok(());
    }

    let brew = homebrew().ok_or(
        "Apple Container is not installed and Homebrew was not found. Install Homebrew, then try again.",
    )?;
    let output = Command::new(&brew)
        .args(["install", "container"])
        .output()
        .map_err(|error| format!("Could not run Homebrew: {error}"))?;
    if !output.status.success() {
        return Err(command_error("Installing Apple Container", &output));
    }
    crate::apple::apple_container_available()
        .then_some(())
        .ok_or_else(|| "Apple Container was installed but could not be started.".to_string())
}

/// Resolve the runtime base dir from a Colima binary path
/// e.g. .../runtime/colima/bin/colima → .../runtime
fn runtime_base_from_colima(colima_path: &Path) -> Option<PathBuf> {
    colima_path.parent()?.parent()?.parent().map(PathBuf::from)
}

/// Build environment variables for Colima, derived from the known binary path
fn colima_env_for(colima_path: &Path) -> Vec<(String, String)> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut env = Vec::new();
    let runtime_base = runtime_base_from_colima(colima_path).filter(|base| {
        base.join("lima/bin/limactl").exists() && base.join("docker/bin/docker").exists()
    });

    // Historical bundled releases stored the VM directly under ~/.lima.
    // Keep using that VM when present; a modern Homebrew-only setup otherwise
    // retains Colima's default ~/.colima/_lima location.
    if runtime_base.is_some() || home.join(".lima/colima").exists() {
        env.push((
            "LIMA_HOME".to_string(),
            home.join(".lima").to_string_lossy().to_string(),
        ));
    }

    // A bundled Colima lives in runtime/colima/bin and has matching sibling
    // lima/docker directories. Homebrew Colima must use Homebrew's own paths;
    // setting LIMA_DIR to a guessed bundle path prevents it from starting.
    if let Some(runtime_base) = runtime_base {
        let lima_dir = runtime_base.join("lima");
        env.push((
            "PATH".to_string(),
            format!(
                "{}:{}:{}:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin",
                lima_dir.join("bin").display(),
                runtime_base.join("colima/bin").display(),
                runtime_base.join("docker/bin").display(),
            ),
        ));
        env.push((
            "LIMA_DIR".to_string(),
            lima_dir.to_string_lossy().to_string(),
        ));
    } else {
        env.push((
            "PATH".to_string(),
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".to_string(),
        ));
    }

    env
}

fn limactl_for(colima_path: &Path) -> Option<PathBuf> {
    runtime_base_from_colima(colima_path)
        .map(|base| base.join("lima/bin/limactl"))
        .filter(|path| path.is_file())
        .or_else(|| {
            [
                PathBuf::from("/opt/homebrew/bin/limactl"),
                PathBuf::from("/usr/local/bin/limactl"),
            ]
            .into_iter()
            .find(|path| path.is_file())
        })
}

fn wait_for_colima_socket() -> bool {
    let socket = colima_socket_path();
    for _ in 0..40 {
        if socket.exists() && docker_socket_responds(&socket) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

/// Detect current runtime status for the selected provider.
///
/// For `Docker`/`Colima` this inspects Docker sockets; for `Apple` it checks
/// the `container` binary and a quick readiness probe. The detection result
/// is independent of the user's *choice* — it reports whether that provider
/// is currently usable.
pub fn detect_runtime(resource_dir: &Path, provider: ProviderKind) -> RuntimeStatus {
    match provider {
        ProviderKind::Apple => detect_apple(),
        ProviderKind::Docker => detect_external_docker(resource_dir),
        ProviderKind::Colima => detect_colima(resource_dir),
    }
}

pub fn provider_status(resource_dir: &Path, provider: ProviderKind) -> ProviderStatus {
    match provider {
        ProviderKind::Docker => {
            let socket = external_docker_socket();
            let installed = docker_cli().is_some()
                || Path::new("/Applications/Docker.app").exists()
                || dirs::home_dir()
                    .unwrap_or_default()
                    .join(".orbstack")
                    .exists();
            ProviderStatus {
                provider,
                installed,
                running: socket.is_some(),
                compatible: true,
                detail: socket
                    .map(|path| format!("Connected at {}", path.display()))
                    .unwrap_or_else(|| {
                        if installed {
                            "Installed, but the Docker engine is stopped".to_string()
                        } else {
                            "Docker Desktop or OrbStack was not detected".to_string()
                        }
                    }),
            }
        }
        ProviderKind::Colima => {
            let installed = bundled_colima(resource_dir).is_some();
            let socket = colima_socket_path();
            let running = socket.exists() && docker_socket_responds(&socket);
            let instance_exists = dirs::home_dir()
                .unwrap_or_default()
                .join(".lima/colima")
                .exists();
            ProviderStatus {
                provider,
                installed,
                running,
                compatible: cfg!(target_os = "macos"),
                detail: if running {
                    "Colima VM is running".to_string()
                } else if instance_exists && !installed {
                    "Existing VM found; the Colima CLI will be restored".to_string()
                } else if installed {
                    "Installed and ready to start".to_string()
                } else {
                    "Not installed; Homebrew will install Colima".to_string()
                },
            }
        }
        ProviderKind::Apple => {
            let installed = crate::apple::apple_container_available();
            let running = installed && crate::apple::system_running();
            let compatible = supported_apple_container_host();
            ProviderStatus {
                provider,
                installed,
                running,
                compatible,
                detail: if !compatible {
                    "Requires Apple Silicon and macOS 26 or later".to_string()
                } else if running {
                    "Apple Container is running".to_string()
                } else if installed {
                    "Installed and ready to start".to_string()
                } else {
                    "Not installed; Homebrew will install Apple Container".to_string()
                },
            }
        }
    }
}

fn detect_external_docker(_resource_dir: &Path) -> RuntimeStatus {
    if external_docker_available() {
        return RuntimeStatus {
            kind: RuntimeKind::External,
            running: true,
            message: "External Docker runtime detected".to_string(),
            provider: ProviderKind::Docker,
        };
    }

    RuntimeStatus {
        kind: RuntimeKind::None,
        running: false,
        message: "External Docker is not running".to_string(),
        provider: ProviderKind::Docker,
    }
}

fn detect_colima(resource_dir: &Path) -> RuntimeStatus {
    let socket = colima_socket_path();
    if socket.exists() && docker_socket_responds(&socket) {
        return RuntimeStatus {
            kind: RuntimeKind::Builtin,
            running: true,
            message: "Built-in runtime (Colima) is running".to_string(),
            provider: ProviderKind::Colima,
        };
    }

    if bundled_colima(resource_dir).is_some() {
        return RuntimeStatus {
            kind: RuntimeKind::Builtin,
            running: false,
            message: "Built-in runtime (Colima) is stopped".to_string(),
            provider: ProviderKind::Colima,
        };
    }

    RuntimeStatus {
        kind: RuntimeKind::None,
        running: false,
        message: "No Docker runtime found".to_string(),
        provider: ProviderKind::Colima,
    }
}

fn detect_apple() -> RuntimeStatus {
    if !crate::apple::apple_container_available() {
        return RuntimeStatus {
            kind: RuntimeKind::None,
            running: false,
            message: "Apple Container CLI not found. Install with: brew install container"
                .to_string(),
            provider: ProviderKind::Apple,
        };
    }

    if crate::apple::system_running() {
        RuntimeStatus {
            kind: RuntimeKind::Apple,
            running: true,
            message: "Apple Container is running".to_string(),
            provider: ProviderKind::Apple,
        }
    } else {
        RuntimeStatus {
            kind: RuntimeKind::Apple,
            running: false,
            message: "Apple Container is stopped".to_string(),
            provider: ProviderKind::Apple,
        }
    }
}

/// Get the Colima docker socket path
pub fn colima_socket_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".colima/default/docker.sock")
}

fn connect_socket(socket: &Path) -> Option<Docker> {
    let url = format!("unix://{}", socket.display());
    Docker::connect_with_unix(&url, 120, bollard::API_DEFAULT_VERSION).ok()
}

/// Connect only to the runtime explicitly selected by the user.
pub fn connect_provider(provider: ProviderKind) -> Option<Docker> {
    match provider {
        ProviderKind::Docker => external_docker_socket().and_then(|path| connect_socket(&path)),
        ProviderKind::Colima => {
            let socket = colima_socket_path();
            (socket.exists() && docker_socket_responds(&socket))
                .then(|| connect_socket(&socket))
                .flatten()
        }
        ProviderKind::Apple => None,
    }
}

pub fn docker_host_for(provider: ProviderKind) -> Option<String> {
    match provider {
        ProviderKind::Docker => external_docker_socket(),
        ProviderKind::Colima => Some(colima_socket_path()),
        ProviderKind::Apple => None,
    }
    .map(|path| format!("unix://{}", path.display()))
}

/// Extract a clean error message from Colima's verbose log output
fn extract_error(full: &str) -> String {
    let error_lines: Vec<&str> = full
        .lines()
        .filter(|l| l.contains("level=fatal") || l.contains("level=error"))
        .collect();
    let raw = if error_lines.is_empty() {
        full.lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Unknown error")
            .to_string()
    } else {
        error_lines.last().unwrap_or(&"Unknown error").to_string()
    };
    // Extract just the msg="..." part if present
    if let Some(idx) = raw.find("msg=") {
        raw[idx + 4..].trim_matches('"').to_string()
    } else {
        raw
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmConfig {
    pub cpu: u32,
    pub memory: u32,
    pub disk: u32,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            cpu: 2,
            memory: 4,
            disk: 20,
        }
    }
}

/// Read current VM config from Lima's YAML (authoritative) or Colima's YAML (fallback)
pub fn read_vm_config() -> VmConfig {
    let home = dirs::home_dir().unwrap_or_default();
    let mut config = VmConfig::default();

    // Try Lima config first (actual running VM values)
    let lima_config = home.join(".lima/colima/lima.yaml");
    if let Ok(content) = std::fs::read_to_string(&lima_config) {
        for line in content.lines() {
            if line.starts_with('#') {
                continue;
            }
            if let Some(val) = line.strip_prefix("cpus:") {
                if let Ok(v) = val.trim().parse::<u32>() {
                    config.cpu = v;
                }
            } else if let Some(val) = line.strip_prefix("memory:") {
                // Lima format: "memory: 4096MiB"
                let val = val.trim().trim_end_matches("MiB").trim_end_matches("GiB");
                if let Ok(v) = val.parse::<u32>() {
                    config.memory = if v >= 1024 { v / 1024 } else { v };
                }
            } else if let Some(val) = line.strip_prefix("disk:") {
                // Lima format: "disk: 20GiB"
                let val = val.trim().trim_end_matches("GiB").trim_end_matches("MiB");
                if let Ok(v) = val.parse::<u32>() {
                    config.disk = v;
                }
            }
        }
        return config;
    }

    // Fallback to Colima config
    let colima_config = home.join(".colima/default/colima.yaml");
    if let Ok(content) = std::fs::read_to_string(&colima_config) {
        for line in content.lines() {
            if line.starts_with('#') {
                continue;
            }
            if let Some(val) = line.strip_prefix("cpu:") {
                if let Ok(v) = val.trim().parse::<u32>() {
                    config.cpu = v;
                }
            } else if let Some(val) = line.strip_prefix("memory:") {
                if let Ok(v) = val.trim().parse::<u32>() {
                    config.memory = v;
                }
            } else if let Some(val) = line.strip_prefix("disk:") {
                if let Ok(v) = val.trim().parse::<u32>() {
                    config.disk = v;
                }
            }
        }
    }

    config
}

/// Write VM config to both Colima and Lima config files
fn write_vm_config(config: &VmConfig) {
    let home = dirs::home_dir().unwrap_or_default();

    // 1. Update Colima config (~/.colima/default/colima.yaml)
    let colima_config = home.join(".colima/default/colima.yaml");
    if let Ok(content) = std::fs::read_to_string(&colima_config) {
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        for line in &mut lines {
            if !line.starts_with('#') {
                if line.starts_with("cpu:") {
                    *line = format!("cpu: {}", config.cpu);
                } else if line.starts_with("memory:") {
                    *line = format!("memory: {}", config.memory);
                } else if line.starts_with("disk:") {
                    *line = format!("disk: {}", config.disk);
                }
            }
        }
        let _ = std::fs::write(&colima_config, lines.join("\n") + "\n");
    }

    // 2. Update Lima config (~/.lima/colima/lima.yaml)
    //    Lima uses different format: cpus, memory in MiB, disk in GiB
    let lima_config = home.join(".lima/colima/lima.yaml");
    if let Ok(content) = std::fs::read_to_string(&lima_config) {
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        for line in &mut lines {
            if !line.starts_with('#') {
                if line.starts_with("cpus:") {
                    *line = format!("cpus: {}", config.cpu);
                } else if line.starts_with("memory:") {
                    *line = format!("memory: {}MiB", config.memory * 1024);
                } else if line.starts_with("disk:") {
                    *line = format!("disk: {}GiB", config.disk);
                }
            }
        }
        let _ = std::fs::write(&lima_config, lines.join("\n") + "\n");
    }
}

/// Start the bundled Colima runtime
pub fn start_builtin(resource_dir: &Path) -> Result<String, String> {
    // Preserve the user's current Colima allocation. In particular, a
    // previously enlarged disk cannot be shrunk by Colima, so restarting with
    // the hard-coded defaults would make the runtime fail to start.
    start_builtin_with_config(resource_dir, &read_vm_config())
}

pub fn start_builtin_with_config(resource_dir: &Path, config: &VmConfig) -> Result<String, String> {
    let colima = ensure_colima(resource_dir)?;

    // Update config files so existing VMs pick up the new values
    write_vm_config(config);

    // Derive env from the actual colima path (not resource_dir, which may be wrong in dev)
    let env = colima_env_for(&colima);
    let cpu = config.cpu.to_string();
    let mem = config.memory.to_string();
    let disk = config.disk.to_string();

    let start = || {
        Command::new(&colima)
            .args([
                "start",
                "--cpu",
                &cpu,
                "--memory",
                &mem,
                "--disk",
                &disk,
                "--runtime",
                "docker",
            ])
            .envs(env.clone())
            .output()
            .map_err(|error| error.to_string())
    };

    let output = start()?;

    if !output.status.success() {
        return Err(command_error("Starting Colima", &output));
    }

    if wait_for_colima_socket() {
        return Ok("Runtime started".to_string());
    }

    // A legacy Lima VM can remain alive while its forwarded Docker socket is
    // gone. Colima reports "already running" in that state, so restart only
    // the VM process and let Colima recreate the forwarding. Never delete the
    // instance: its disk contains the user's containers and images.
    let limactl = limactl_for(&colima)
        .ok_or_else(|| "Colima is running, but its Docker socket is unavailable.".to_string())?;
    let stop = Command::new(limactl)
        .args(["stop", "colima"])
        .envs(env.clone())
        .output()
        .map_err(|error| error.to_string())?;
    if !stop.status.success() {
        return Err(command_error("Repairing Colima socket forwarding", &stop));
    }

    let retry = start()?;
    if !retry.status.success() {
        return Err(command_error("Restarting Colima", &retry));
    }
    if !wait_for_colima_socket() {
        return Err("Colima restarted, but its Docker socket did not respond.".to_string());
    }

    Ok("Runtime socket repaired".to_string())
}

/// Stop the bundled Colima runtime
pub fn stop_builtin(resource_dir: &Path) -> Result<String, String> {
    let colima = bundled_colima(resource_dir).ok_or("Bundled Colima not found")?;

    let env = colima_env_for(&colima);

    let output = Command::new(&colima)
        .args(["stop"])
        .envs(env)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let full = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return Err(extract_error(&full));
    }

    Ok("Runtime stopped".to_string())
}
