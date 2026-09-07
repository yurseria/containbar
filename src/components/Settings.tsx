import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Provider, RuntimeOverview } from "../types";
import { providerCapabilities } from "../types";
import { applyAppTheme, type AppTheme } from "../theme";
import { RuntimePicker } from "./RuntimeSetup";

interface SettingsData {
  terminal: string;
  shell: string;
  refreshInterval: number;
  uiScale: number;
  theme: AppTheme;
  /** Active runtime provider (kept in sync with the backend `provider.json`). */
  provider: Provider;
}

const STORAGE_KEY = "docker-tray-settings";
const DEFAULTS: SettingsData = {
  terminal: "auto",
  shell: "/bin/sh",
  refreshInterval: 5,
  uiScale: 1.0,
  theme: "cobalt",
  provider: "docker",
};

function loadSettings(): SettingsData {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...DEFAULTS, ...JSON.parse(saved) };
  } catch {
    // ignore
  }
  return DEFAULTS;
}

function saveSettings(settings: SettingsData) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

export function applyScale(scale: number) {
  const root = document.documentElement;
  root.style.fontSize = `${scale * 13}px`;
}

interface Props {
  onClose: () => void;
  onVmRestart?: () => void;
  runtimeOverview: RuntimeOverview | null;
  runtimeBusy: Provider | null;
  runtimeError: string | null;
  onProviderChange: (provider: Provider) => Promise<void>;
}

interface VmConfig {
  cpu: number;
  memory: number;
  disk: number;
}

interface UpdateInfo {
  current_version: string;
  latest_version: string;
  has_update: boolean;
  release_url: string;
}

type UpdateStatus = "idle" | "checking" | "up-to-date" | "update-available" | "error";
type ThemeStatus = "idle" | "loading" | "success" | "error";

export function Settings({
  onClose,
  onVmRestart,
  runtimeOverview,
  runtimeBusy,
  runtimeError,
  onProviderChange,
}: Props) {
  const [settings, setSettings] = useState<SettingsData>(loadSettings);
  const [detectedTerminal, setDetectedTerminal] = useState("...");
  const [autostart, setAutostart] = useState(false);
  const [vmConfig, setVmConfig] = useState<VmConfig | null>(null);
  const [vmDraft, setVmDraft] = useState<VmConfig | null>(null);
  const [vmApplying, setVmApplying] = useState(false);
  const [appVersion, setAppVersion] = useState("");
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [themeStatus, setThemeStatus] = useState<ThemeStatus>("idle");
  const [themeTarget, setThemeTarget] = useState<AppTheme | null>(null);

  useEffect(() => {
    if (themeStatus !== "success" && themeStatus !== "error") return;
    const timeout = window.setTimeout(() => {
      setThemeStatus("idle");
      setThemeTarget(null);
    }, themeStatus === "success" ? 900 : 1800);
    return () => window.clearTimeout(timeout);
  }, [themeStatus]);

  const handleCheckUpdate = async () => {
    setUpdateStatus("checking");
    try {
      const info = await invoke<UpdateInfo>("check_for_updates");
      setUpdateInfo(info);
      setAppVersion(info.current_version);
      setUpdateStatus(info.has_update ? "update-available" : "up-to-date");
    } catch {
      setUpdateStatus("error");
    }
  };

  useEffect(() => {
    invoke<string>("get_app_version").then(setAppVersion).catch(() => {});
    invoke<boolean>("get_autostart").then(setAutostart).catch(() => {});
    // Sync the persisted provider from the backend (source of truth).
    invoke<Provider>("get_provider")
      .then((p) => update({ provider: p }))
      .catch(() => {});
    invoke<string>("detect_terminal").then((t) => {
      const names: Record<string, string> = {
        ghostty: "Ghostty",
        iterm: "iTerm2",
        terminal: "Terminal.app",
      };
      setDetectedTerminal(names[t] || t);
    });
    invoke<VmConfig>("get_vm_config").then((c) => {
      setVmConfig(c);
      setVmDraft(c);
    }).catch(() => {});
  }, []);

  const vmChanged = vmConfig && vmDraft && (
    vmDraft.cpu !== vmConfig.cpu ||
    vmDraft.memory !== vmConfig.memory ||
    vmDraft.disk !== vmConfig.disk
  );

  const applyVmConfig = async () => {
    if (!vmDraft || vmApplying) return;
    setVmApplying(true);
    try {
      await invoke("apply_vm_config", { config: vmDraft });
      setVmConfig(vmDraft);
      onVmRestart?.();
    } catch (e) {
      console.error("Failed to apply VM config:", e);
      setVmApplying(false);
    }
  };

  // Provider selection persists to disk (backend is source of truth) and also
  // mirrors into the localStorage settings for quick UI reads.
  const changeProvider = async (provider: Provider) => {
    try {
      await onProviderChange(provider);
      update({ provider });
    } catch (e) {
      console.error("Failed to set provider:", e);
    }
  };

  const caps = providerCapabilities(settings.provider);

  const changeTheme = async (theme: AppTheme) => {
    if (themeStatus === "loading" || theme === settings.theme) return;
    const previous = settings.theme;
    update({ theme });
    setThemeTarget(theme);
    setThemeStatus("loading");
    try {
      await applyAppTheme(theme);
      setThemeStatus("success");
    } catch (error) {
      console.error("Failed to apply app theme:", error);
      update({ theme: previous });
      await applyAppTheme(previous).catch(() => {});
      setThemeStatus("error");
    }
  };

  const update = (partial: Partial<SettingsData>) => {
    setSettings((prev) => {
      const next = { ...prev, ...partial };
      saveSettings(next);
      if (partial.uiScale !== undefined) {
        applyScale(partial.uiScale);
      }
      return next;
    });
  };

  return (
    <div className="settings">
      <div className="settings-header">
        <span className="settings-title">Settings</span>
        <button className="action-btn" onClick={onClose} title="Close">
          <i className="ri-close-line" />
        </button>
      </div>

      <div className="settings-content">
        <div className="settings-group settings-runtime">
          <label className="settings-label">Runtime</label>
          {runtimeOverview ? (
            <RuntimePicker
              overview={runtimeOverview}
              busyProvider={runtimeBusy}
              error={runtimeError}
              compact
              onSelect={changeProvider}
            />
          ) : (
            <span className="settings-hint">Checking installed runtimes…</span>
          )}
        </div>

        <div className="settings-group">
          <label className="settings-label">Appearance</label>
          <div className="theme-options" role="group" aria-label="App theme">
            {[
              { value: "cobalt" as const, label: "Cobalt", icon: "ri-moon-clear-line" },
              { value: "liquid-glass" as const, label: "Liquid Glass", icon: "ri-water-flash-line" },
            ].map((option) => {
              const active = settings.theme === option.value;
              const state = themeTarget === option.value && themeStatus !== "idle" ? themeStatus : undefined;
              return (
                <button
                  key={option.value}
                  className={`theme-option ${active ? "active" : ""}`}
                  type="button"
                  aria-pressed={active}
                  data-state={state}
                  disabled={themeStatus === "loading"}
                  onClick={() => changeTheme(option.value)}
                >
                  <i className={state === "loading" ? "ri-loader-4-line" : option.icon} aria-hidden="true" />
                  <span>{option.label}</span>
                  {state === "success" && <i className="ri-check-line theme-option-state" aria-hidden="true" />}
                  {state === "error" && <i className="ri-error-warning-line theme-option-state" aria-hidden="true" />}
                </button>
              );
            })}
          </div>
          <span className={`settings-hint ${themeStatus === "error" ? "theme-hint--error" : ""}`}>
            {themeStatus === "error"
              ? "Liquid Glass could not be applied. Cobalt was restored."
              : "Liquid Glass uses the native macOS material and falls back to vibrancy on older systems."}
          </span>
        </div>

        <div className="settings-group">
          <label className="settings-label">UI Scale</label>
          <div className="scale-options">
            {[
              { value: 0.9, label: "S" },
              { value: 1.0, label: "M" },
              { value: 1.1, label: "L" },
              { value: 1.2, label: "XL" },
            ].map((opt) => (
              <button
                key={opt.value}
                className={`scale-btn ${settings.uiScale === opt.value ? "active" : ""}`}
                onClick={() => update({ uiScale: opt.value })}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        <div className="settings-group">
          <label className="settings-label">Terminal</label>
          <select
            className="settings-select"
            value={settings.terminal}
            onChange={(e) => update({ terminal: e.target.value })}
          >
            <option value="auto">Auto-detect ({detectedTerminal})</option>
            <option value="ghostty">Ghostty</option>
            <option value="iterm">iTerm2</option>
            <option value="terminal">Terminal.app</option>
          </select>
        </div>

        <div className="settings-group">
          <label className="settings-label">Shell</label>
          <select
            className="settings-select"
            value={settings.shell}
            onChange={(e) => update({ shell: e.target.value })}
          >
            <option value="/bin/sh">/bin/sh</option>
            <option value="/bin/bash">/bin/bash</option>
            <option value="/bin/zsh">/bin/zsh</option>
          </select>
          <span className="settings-hint">Shell used for docker exec</span>
        </div>

        <div className="settings-group">
          <label className="settings-label">Refresh interval</label>
          <select
            className="settings-select"
            value={settings.refreshInterval}
            onChange={(e) =>
              update({ refreshInterval: Number(e.target.value) })
            }
          >
            <option value={2}>2 seconds</option>
            <option value={5}>5 seconds</option>
            <option value={10}>10 seconds</option>
            <option value={30}>30 seconds</option>
          </select>
        </div>

        <div className="settings-group">
          <label className="settings-label">Start at Login</label>
          <label className="settings-toggle">
            <input
              type="checkbox"
              checked={autostart}
              onChange={(e) => {
                const val = e.target.checked;
                invoke("set_autostart", { enabled: val }).then(() => setAutostart(val)).catch(() => {});
              }}
            />
            <span className="settings-hint">Launch Docker Tray when you log in</span>
          </label>
        </div>

        {caps.vmResources && vmDraft && (
          <>
            <div className="settings-divider" />
            <div className="settings-section-title">VM Resources</div>
            <div className="settings-group">
              <label className="settings-label">CPU (cores)</label>
              <select
                className="settings-select"
                value={vmDraft.cpu}
                onChange={(e) => setVmDraft({ ...vmDraft, cpu: Number(e.target.value) })}
                disabled={vmApplying}
              >
                {[1, 2, 3, 4, 6, 8].map((v) => (
                  <option key={v} value={v}>{v}</option>
                ))}
              </select>
            </div>
            <div className="settings-group">
              <label className="settings-label">Memory (GB)</label>
              <select
                className="settings-select"
                value={vmDraft.memory}
                onChange={(e) => setVmDraft({ ...vmDraft, memory: Number(e.target.value) })}
                disabled={vmApplying}
              >
                {[1, 2, 4, 6, 8, 12, 16].map((v) => (
                  <option key={v} value={v}>{v}</option>
                ))}
              </select>
            </div>
            <div className="settings-group">
              <label className="settings-label">Disk (GB)</label>
              <select
                className="settings-select"
                value={vmDraft.disk}
                onChange={(e) => setVmDraft({ ...vmDraft, disk: Number(e.target.value) })}
                disabled={vmApplying}
              >
                {[10, 20, 40, 60, 80, 100].map((v) => (
                  <option key={v} value={v}>{v}</option>
                ))}
              </select>
            </div>
            {vmChanged && (
              <button
                className="confirm-btn danger vm-apply-btn"
                onClick={applyVmConfig}
                disabled={vmApplying}
              >
                {vmApplying ? "Restarting VM..." : "Apply & Restart VM"}
              </button>
            )}
            <span className="settings-hint">Changing VM settings will restart the runtime</span>
          </>
        )}

        <div className="settings-group">
          <label className="settings-label">About</label>
          <span className="settings-hint">Docker Tray{appVersion ? ` v${appVersion}` : ""}</span>
          <div className="update-check-row">
            <button
              className="update-check-btn"
              onClick={handleCheckUpdate}
              disabled={updateStatus === "checking"}
            >
              {updateStatus === "checking" ? "Checking..." : "Check for Updates"}
            </button>
            {updateStatus === "up-to-date" && (
              <span className="settings-hint update-status">Up to date</span>
            )}
            {updateStatus === "error" && (
              <span className="settings-hint update-status update-status--error">Check failed</span>
            )}
            {updateStatus === "update-available" && updateInfo && (
              <button
                className="update-status update-status--available"
                onClick={() => openUrl(updateInfo.release_url)}
              >
                v{updateInfo.latest_version} available →
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export function getSettings(): SettingsData {
  return loadSettings();
}
