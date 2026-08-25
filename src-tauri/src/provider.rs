use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

/// The container runtime the user has chosen to drive.
///
/// This is distinct from `RuntimeKind` (a *detection* result): `ProviderKind`
/// is the user's intent, persisted in `provider.json`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Use an external Docker Engine socket (Docker Desktop / OrbStack).
    /// This never falls back to Colima; provider selection is explicit.
    #[default]
    Docker,
    /// Drive Apple's native `container` CLI (macOS 26+, Apple Silicon).
    Apple,
    /// Force the bundled Colima VM even if an external Docker socket exists.
    Colima,
}

impl ProviderKind {
    /// True when this provider is driven through the Docker Engine API
    /// (Bollard). Both `Docker` (external socket) and `Colima` (bundled VM
    /// exposing a docker.sock) share the same code path.
    #[allow(dead_code)]
    pub fn uses_docker_api(self) -> bool {
        matches!(self, ProviderKind::Docker | ProviderKind::Colima)
    }
}

/// In-memory cache of the selected provider, so command dispatch can read it
/// without touching disk on every call.
pub struct ProviderState(pub Arc<Mutex<ProviderKind>>);

impl ProviderState {
    pub fn new(initial: ProviderKind) -> Self {
        ProviderState(Arc::new(Mutex::new(initial)))
    }

    pub fn get(&self) -> ProviderKind {
        *self.0.lock().expect("provider mutex poisoned")
    }

    pub fn set(&self, provider: ProviderKind) {
        *self.0.lock().expect("provider mutex poisoned") = provider;
    }
}

/// Path to the persisted provider config: `<app_config_dir>/provider.json`.
fn provider_config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("provider.json"))
}

/// Whether the user has completed runtime selection at least once.
///
/// Older builds silently defaulted to Docker when this file was absent. The
/// setup wizard now uses a valid file as the durable first-run marker.
pub fn setup_complete(app: &AppHandle) -> bool {
    provider_config_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|content| serde_json::from_str::<ProviderFile>(&content).ok())
        .is_some()
}

#[derive(Serialize, Deserialize)]
struct ProviderFile {
    #[serde(default)]
    provider: ProviderKind,
}

/// Load the persisted provider, falling back to the default on any error.
pub fn load_provider(app: &AppHandle) -> ProviderKind {
    let Some(path) = provider_config_path(app) else {
        return ProviderKind::default();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return ProviderKind::default();
    };
    serde_json::from_str::<ProviderFile>(&content)
        .map(|f| f.provider)
        .unwrap_or_default()
}

/// Persist the provider selection to disk.
pub fn store_provider(app: &AppHandle, provider: ProviderKind) -> Result<(), String> {
    let path = provider_config_path(app).ok_or_else(|| "Cannot resolve config dir".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body =
        serde_json::to_string_pretty(&ProviderFile { provider }).map_err(|e| e.to_string())?;
    std::fs::write(&path, body).map_err(|e| e.to_string())
}
