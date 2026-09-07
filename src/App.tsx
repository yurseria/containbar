import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useDocker } from "./hooks/useDocker";
import { useAutoResize } from "./hooks/useAutoResize";
import { ContainersTab } from "./components/ContainersTab";
import { ImagesTab } from "./components/ImagesTab";
import { VolumesTab } from "./components/VolumesTab";
import { NetworksTab } from "./components/NetworksTab";
import { Settings, getSettings } from "./components/Settings";
import { PullImageDialog } from "./components/PullImageDialog";
import { CreateContainerDialog } from "./components/CreateContainerDialog";
import { ComposeDialog } from "./components/ComposeDialog";
import { RuntimeSetup } from "./components/RuntimeSetup";
import type { Tab, Provider, RuntimeOverview } from "./types";
import { providerCapabilities } from "./types";
import "./App.css";

type DialogType = "pull" | "create" | "compose" | null;

const TABS: { key: Tab; label: string; iconClass: string }[] = [
  { key: "containers", label: "Containers", iconClass: "ri-instance-line" },
  { key: "images", label: "Images", iconClass: "ri-stack-line" },
  { key: "volumes", label: "Volumes", iconClass: "ri-hard-drive-3-line" },
  { key: "networks", label: "Networks", iconClass: "ri-share-line" },
];

function App() {
  const [activeTab, setActiveTab] = useState<Tab>("containers");
  const [showSettings, setShowSettings] = useState(false);
  const [showRuntimeManager, setShowRuntimeManager] = useState(false);
  const [dialog, setDialog] = useState<DialogType>(null);
  const [createImage, setCreateImage] = useState<string | undefined>();
  const [search, setSearch] = useState("");
  const [provider, setProvider] = useState<Provider>("docker");
  const [runtimeOverview, setRuntimeOverview] = useState<RuntimeOverview | null>(null);
  const [runtimeOverviewError, setRuntimeOverviewError] = useState<string | null>(null);
  const [busyProvider, setBusyProvider] = useState<Provider | null>(null);

  const caps = providerCapabilities(provider);

  const appRef = useRef<HTMLDivElement>(null);
  useAutoResize(appRef);

  const docker = useDocker();
  const { fetchContainers, fetchImages, fetchVolumes, fetchNetworks, ping } = docker;

  const fetchCurrentTab = useCallback(() => {
    switch (activeTab) {
      case "containers": fetchContainers(); break;
      case "images": fetchImages(); break;
      case "volumes": fetchVolumes(); break;
      case "networks": fetchNetworks(); break;
    }
  }, [activeTab, fetchContainers, fetchImages, fetchVolumes, fetchNetworks]);

  useEffect(() => {
    ping();
    fetchCurrentTab();
  }, [activeTab, ping, fetchCurrentTab]);

  // Keep the active provider in sync with the backend (source of truth).
  useEffect(() => {
    invoke<Provider>("get_provider")
      .then(setProvider)
      .catch(() => {});
  }, []);

  const refreshRuntimeOverview = useCallback(async () => {
    try {
      const overview = await invoke<RuntimeOverview>("runtime_overview");
      setRuntimeOverview(overview);
      setProvider(overview.selected);
      setRuntimeOverviewError(null);
    } catch (error) {
      setRuntimeOverviewError(String(error));
    }
  }, []);

  useEffect(() => {
    refreshRuntimeOverview();
  }, [refreshRuntimeOverview]);

  useEffect(() => {
    const ms = getSettings().refreshInterval * 1000;
    const interval = setInterval(fetchCurrentTab, ms);
    return () => clearInterval(interval);
  }, [fetchCurrentTab]);

  const [runtimeLoading, setRuntimeLoading] = useState(false);
  const [runtimeStatus, setRuntimeStatus] = useState("");
  const [runtimeError, setRuntimeError] = useState<string | null>(null);
  const [runtimeHint, setRuntimeHint] = useState("No Docker runtime detected");

  // Check runtime status on mount when disconnected
  useEffect(() => {
    if (!docker.connected) {
      invoke<{ kind: string; running: boolean; message: string }>("runtime_status")
        .then((s) => {
          setRuntimeHint(s.message);
          // If already starting in background, show spinner and start polling
          if (s.message === "Starting runtime...") {
            setRuntimeLoading(true);
            setRuntimeStatus("Starting VM...");
            startPolling();
          }
        })
        .catch(() => {});
    }
  }, [docker.connected]);

  const startPolling = () => {
    const msgs = ["Starting VM...", "Downloading VM image (first run only)...", "Configuring Docker engine...", "Almost ready..."];
    let msgIdx = 0;
    const msgTimer = setInterval(() => {
      msgIdx = Math.min(msgIdx + 1, msgs.length - 1);
      setRuntimeStatus(msgs[msgIdx]);
    }, 8000);

    const poll = setInterval(async () => {
      try {
        const status = await invoke<{ kind: string; running: boolean; message: string }>("runtime_status");
        if (status.message === "Starting runtime...") {
          return; // Still starting
        }
        clearInterval(poll);
        clearInterval(msgTimer);

        if (status.running) {
          setRuntimeStatus("Connecting...");
          // Retry ping until Docker is ready (socket may take a moment)
          for (let i = 0; i < 5; i++) {
            await new Promise((r) => setTimeout(r, 1500));
            if (await ping()) {
              setRuntimeLoading(false);
              setRuntimeStatus("");
              return;
            }
          }
          setRuntimeError("Runtime started but Docker connection failed. Try again.");
        } else {
          setRuntimeError(status.message || "Failed to start runtime");
        }
        setRuntimeLoading(false);
        setRuntimeStatus("");
      } catch {
        // Keep polling
      }
    }, 2000);
  };

  const startRuntime = async () => {
    setRuntimeLoading(true);
    setRuntimeError(null);
    setRuntimeStatus("Starting VM...");

    try {
      await invoke("runtime_start");
    } catch (e) {
      setRuntimeError(String(e));
      setRuntimeLoading(false);
      return;
    }

    startPolling();
  };

  const switchRuntime = async (nextProvider: Provider) => {
    if (busyProvider) return;
    setBusyProvider(nextProvider);
    setRuntimeOverviewError(null);
    docker.disconnect();
    try {
      const overview = await invoke<RuntimeOverview>("switch_provider", { provider: nextProvider });
      setRuntimeOverview(overview);
      setProvider(overview.selected);
      if (!(await ping())) {
        throw new Error(`${nextProvider} started, but the app could not connect.`);
      }
      fetchCurrentTab();
      setShowSettings(false);
      setShowRuntimeManager(false);
    } catch (error) {
      await refreshRuntimeOverview();
      setRuntimeOverviewError(String(error));
      await ping();
      throw error;
    } finally {
      setBusyProvider(null);
    }
  };

  if (!runtimeOverview) {
    return (
      <div className="app" ref={appRef}>
        <div className="runtime-boot" role="status">
          <div className="runtime-spinner" />
          <span>{runtimeOverviewError || "Checking installed runtimes…"}</span>
        </div>
      </div>
    );
  }

  if (!runtimeOverview.setup_complete) {
    return (
      <div className="app runtime-app" ref={appRef}>
        <div className="titlebar">
          <span className="titlebar-text">Docker Tray</span>
        </div>
        <RuntimeSetup
          overview={runtimeOverview}
          busyProvider={busyProvider}
          error={runtimeOverviewError}
          onSelect={switchRuntime}
        />
      </div>
    );
  }

  if (!docker.connected) {
    if (showRuntimeManager) {
      return (
        <div className="app runtime-app" ref={appRef}>
          <div className="titlebar">
            <button
              className="settings-btn"
              onClick={() => setShowRuntimeManager(false)}
              title="Back"
            >
              <i className="ri-arrow-left-line" />
            </button>
            <span className="titlebar-text">Runtimes</span>
          </div>
          <RuntimeSetup
            overview={runtimeOverview}
            busyProvider={busyProvider}
            error={runtimeOverviewError}
            onSelect={switchRuntime}
          />
        </div>
      );
    }
    return (
      <div className="app" ref={appRef}>
        <div className="titlebar">
          <span className="titlebar-text">Docker Tray</span>
          <button
            className="settings-btn"
            onClick={() => { refreshRuntimeOverview(); setShowRuntimeManager(true); }}
            title="Change Runtime"
          >
            <i className="ri-instance-line" />
          </button>
        </div>
        <div className="disconnected">
          {runtimeLoading ? (
            <>
              <div className="runtime-spinner" />
              <div className="disconnected-text">Starting Runtime</div>
              <div className="disconnected-hint">{runtimeStatus}</div>
            </>
          ) : (
            <>
              <i className="ri-server-line disconnected-icon" />
              <div className="disconnected-text">Runtime not available</div>
              <div className="disconnected-hint">{runtimeHint}</div>
            </>
          )}
          {runtimeError && <div className="disconnected-error">{runtimeError}</div>}
          {!runtimeLoading && (
            <div className="disconnected-actions">
              {provider !== "docker" ? (
                <button className="retry-btn primary" onClick={startRuntime}>
                  <i className="ri-play-fill" /> Start Runtime
                </button>
              ) : (
                <button className="retry-btn" onClick={ping}>
                  <i className="ri-refresh-line" /> Retry Connection
                </button>
              )}
              <button className="retry-btn" onClick={() => { refreshRuntimeOverview(); setShowRuntimeManager(true); }}>
                <i className="ri-instance-line" /> Change Runtime
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="app" ref={appRef}>
      <div className="titlebar glass-surface chrome-glass-surface">
        <span className="titlebar-text">Docker Tray</span>
        {docker.loading && <span className="loading-indicator" />}
        <button
          className="settings-btn"
          onClick={() => setDialog("pull")}
          title="Pull Image"
        >
          <i className="ri-download-line" />
        </button>
        <button
          className="settings-btn"
          onClick={() => { fetchImages(); setDialog("create"); }}
          title="Create Container"
        >
          <i className="ri-add-box-line" />
        </button>
        {caps.compose && (
          <button
            className="settings-btn"
            onClick={() => setDialog("compose")}
            title="Compose Up"
          >
            <i className="ri-file-upload-line" />
          </button>
        )}
        <button
          className="settings-btn"
          onClick={() => {
            refreshRuntimeOverview();
            setShowSettings((v) => !v);
          }}
          title="Settings"
        >
          <i className="ri-settings-3-line" />
        </button>
      </div>

      {showSettings ? (
        <Settings
          onClose={() => setShowSettings(false)}
          runtimeOverview={runtimeOverview}
          runtimeBusy={busyProvider}
          runtimeError={runtimeOverviewError}
          onVmRestart={() => {
            setShowSettings(false);
            setRuntimeLoading(true);
            setRuntimeError(null);
            setRuntimeStatus("Restarting VM...");
            docker.disconnect();
            startPolling();
          }}
          onProviderChange={switchRuntime}
        />
      ) : (
        <>
          <nav className="tabs glass-surface chrome-glass-surface">
            {TABS.map((tab) => (
              <button
                key={tab.key}
                className={`tab ${activeTab === tab.key ? "active" : ""}`}
                onClick={() => setActiveTab(tab.key)}
              >
                <i className={`tab-icon ${tab.iconClass}`} />
                <span className="tab-label">{tab.label}</span>
              </button>
            ))}
          </nav>

          <div className="search-bar">
            <i className="ri-search-line search-icon" />
            <input
              className="search-input"
              placeholder="Search..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            {search && (
              <button className="search-clear" onClick={() => setSearch("")}>
                <i className="ri-close-line" />
              </button>
            )}
          </div>

          <div className="content">
            {docker.error && (
              <div className="error-banner">
                <i className="ri-error-warning-line" /> {docker.error}
              </div>
            )}

            {activeTab === "containers" && (
              <ContainersTab
                groups={docker.containers}
                search={search}
                onStart={docker.startContainer}
                onStop={docker.stopContainer}
                onRestart={docker.restartContainer}
                onStartGroup={docker.startContainerGroup}
                onStopGroup={docker.stopContainerGroup}
                onRemove={docker.removeContainer}
                getEnv={docker.getContainerEnv}
              />
            )}
            {activeTab === "images" && (
              <ImagesTab
                images={docker.images}
                search={search}
                onRemove={docker.removeImage}
                onCreateContainer={(img) => { setCreateImage(img); setDialog("create"); }}
              />
            )}
            {activeTab === "volumes" && (
              <VolumesTab volumes={docker.volumes} search={search} onRemove={docker.removeVolume} />
            )}
            {activeTab === "networks" && (
              <NetworksTab networks={docker.networks} search={search} onRemove={docker.removeNetwork} />
            )}
          </div>
        </>
      )}
      {dialog === "pull" && (
        <PullImageDialog onPull={docker.pullImage} onClose={() => setDialog(null)} />
      )}
      {dialog === "create" && (
        <CreateContainerDialog
          defaultImage={createImage}
          images={docker.images}
          onSubmit={docker.createContainer}
          onClose={() => { setDialog(null); setCreateImage(undefined); }}
        />
      )}
      {dialog === "compose" && (
        <ComposeDialog onCompose={docker.composeUp} onClose={() => setDialog(null)} />
      )}
    </div>
  );
}

export default App;
