import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

interface Props {
  onCompose: (filePath: string) => Promise<string>;
  onClose: () => void;
}

interface ServiceStatus {
  name: string;
  status: "waiting" | "in-progress" | "done" | "error";
  detail: string;
}

interface ComposePortConflict {
  ports: string[];
  provider: "docker" | "colima" | null;
  project_name: string | null;
  container_count: number;
  can_stop: boolean;
}

const providerName = (provider: ComposePortConflict["provider"]) => {
  if (provider === "colima") return "Colima";
  if (provider === "docker") return "Docker";
  return "Another process";
};

function parseProgressLine(line: string): { name: string; detail: string } | null {
  const mockerStep = line.match(/\[(\d+)\/(\d+)\]\s*(.+)/);
  if (mockerStep) {
    return {
      name: "Mocker",
      detail: `[${mockerStep[1]}/${mockerStep[2]}] ${mockerStep[3].trim()}`,
    };
  }
  if (/\b(?:blobs?|platform linux|fetching|unpacking)\b|\d+%/i.test(line)) {
    return { name: "Image", detail: line.trim() };
  }
  // Docker Compose and Mocker progress patterns:
  //  " Container myapp-db-1  Creating"
  //  " Container myapp-db-1  Started"
  //  " Network myapp_default  Creating"
  const match = line.match(/(?:Container|Network|Volume|Image|Tool)\s+(\S+)\s+(.+)/i);
  if (match) {
    return { name: match[1], detail: match[2].trim() };
  }
  // Pulling lines: " db Pulling", " db Pull complete"
  const pullMatch = line.match(/^\s*(\S+)\s+(Pull.*)$/i);
  if (pullMatch) {
    return { name: pullMatch[1], detail: pullMatch[2].trim() };
  }
  return null;
}

function statusFromDetail(detail: string): ServiceStatus["status"] {
  const d = detail.toLowerCase();
  if (d.includes("error") || d.includes("failed")) return "error";
  if (d.includes("started") || d.includes("running") || d.includes("created") || d.includes("complete") || d.includes("installed") || d.includes("ready") || d.includes("available")) return "done";
  return "in-progress";
}

export function ComposeDialog({ onCompose, onClose }: Props) {
  const [filePath, setFilePath] = useState("");
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !loading) onClose(); };
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [onClose, loading]);
  const [error, setError] = useState<string | null>(null);
  const [services, setServices] = useState<ServiceStatus[]>([]);
  const [conflicts, setConflicts] = useState<ComposePortConflict[]>([]);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  useEffect(() => {
    return () => { unlistenRef.current?.(); };
  }, []);

  useEffect(() => {
    if (!loading) {
      setElapsedSeconds(0);
      return;
    }
    const startedAt = Date.now();
    const timer = window.setInterval(() => {
      setElapsedSeconds(Math.floor((Date.now() - startedAt) / 1000));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [loading]);

  const handleBrowse = async () => {
    try {
      const path = await invoke<string | null>("pick_yaml_file");
      const win = getCurrentWebviewWindow();
      await win.show();
      await win.setFocus();
      if (path) {
        setFilePath(path);
        setConflicts([]);
        setError(null);
        setServices([]);
      }
    } catch {
      const win = getCurrentWebviewWindow();
      await win.show();
      await win.setFocus();
    }
  };

  const runCompose = async (path: string) => {
    // Listen for progress events
    unlistenRef.current = await listen<string>("compose-progress", (event) => {
      const line = event.payload;
      if (line === "done" || line === "error") return;
      const parsed = parseProgressLine(line);
      if (!parsed) return;
      setServices((prev) => {
        const idx = prev.findIndex((s) => s.name === parsed.name);
        const entry: ServiceStatus = {
          name: parsed.name,
          status: statusFromDetail(parsed.detail),
          detail: parsed.detail,
        };
        if (idx >= 0) {
          const next = [...prev];
          next[idx] = entry;
          return next;
        }
        return [...prev, entry];
      });
    });

    try {
      await onCompose(path);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      unlistenRef.current?.();
      unlistenRef.current = null;
      setLoading(false);
    }
  };

  const handleSubmit = async () => {
    const path = filePath.trim();
    if (!path) return;
    setLoading(true);
    setError(null);
    setServices([{ name: "Compose", status: "in-progress", detail: "Checking port availability" }]);
    setConflicts([]);

    try {
      const found = await invoke<ComposePortConflict[]>("inspect_compose_conflicts", { filePath: path });
      if (found.length > 0) {
        setConflicts(found);
        setLoading(false);
        return;
      }
      await runCompose(path);
    } catch (e) {
      setError(String(e));
      setLoading(false);
    }
  };

  const handleStopAndContinue = async () => {
    const path = filePath.trim();
    if (!path) return;
    setLoading(true);
    setError(null);
    setServices([{ name: "Compose", status: "in-progress", detail: "Stopping conflicting project" }]);
    try {
      await invoke("stop_conflicting_compose_projects", { filePath: path });
      setConflicts([]);
      setServices([]);
      await runCompose(path);
    } catch (e) {
      setError(String(e));
      setLoading(false);
    }
  };

  const canStopAll = conflicts.length > 0 && conflicts.every((conflict) => conflict.can_stop);

  return (
    <div className="confirm-overlay">
      <div className="modal-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="modal-title">Compose Up</h3>
        <div className="modal-field">
          <label className="modal-label">docker-compose.yaml</label>
          <div className="modal-browse">
            <input
              className="modal-input"
              placeholder="/path/to/docker-compose.yaml"
              value={filePath}
              onChange={(e) => {
                setFilePath(e.target.value);
                setConflicts([]);
                setError(null);
                setServices([]);
              }}
              onKeyDown={(e) => e.key === "Enter" && conflicts.length === 0 && handleSubmit()}
              disabled={loading}
            />
            <button className="confirm-btn cancel" onClick={handleBrowse} disabled={loading}>
              Browse
            </button>
          </div>
        </div>
        {loading && (
          <div className="compose-running" role="status" aria-live="polite">
            <span className="compose-spinner" />
            <span>Compose is working</span>
            <span className="compose-elapsed">{elapsedSeconds}s</span>
          </div>
        )}
        {services.length > 0 && (
          <div className="compose-progress">
            {services.map((s) => (
              <div key={s.name} className={`compose-progress-item ${s.status}`}>
                <span className="compose-progress-icon">
                  {s.status === "done" ? "\u2713" : s.status === "error" ? "\u2717" : "\u25CB"}
                </span>
                <span className="compose-progress-name">{s.name}</span>
                <span className="compose-progress-detail">{s.detail}</span>
              </div>
            ))}
          </div>
        )}
        {conflicts.length > 0 && (
          <div className="compose-conflict" role="alert">
            <div className="compose-conflict-title">
              <i className="ri-error-warning-line" /> Port conflict
            </div>
            {conflicts.map((conflict, index) => (
              <div className="compose-conflict-item" key={`${conflict.provider}-${conflict.project_name}-${index}`}>
                {conflict.project_name ? (
                  <>
                    <strong>{conflict.project_name}</strong> on {providerName(conflict.provider)} is using {conflict.ports.join(", ")}
                    {conflict.container_count > 0 && ` (${conflict.container_count} containers)`}
                  </>
                ) : (
                  <>Another process is using {conflict.ports.join(", ")}. It cannot be stopped automatically.</>
                )}
              </div>
            ))}
            {canStopAll && (
              <div className="compose-conflict-hint">
                Stop the existing Compose project and continue with Apple Container?
              </div>
            )}
          </div>
        )}
        {error && <div className="modal-error">{error}</div>}
        <div className="confirm-actions">
          <button className="confirm-btn cancel" onClick={onClose} disabled={loading}>
            Cancel
          </button>
          {conflicts.length > 0 ? (
            canStopAll && (
              <button className="confirm-btn danger" onClick={handleStopAndContinue} disabled={loading}>
                {loading ? "Stopping..." : "Stop & Continue"}
              </button>
            )
          ) : (
            <button
              className="confirm-btn primary"
              onClick={handleSubmit}
              disabled={loading || !filePath.trim()}
            >
              {loading ? "Running..." : "Up"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
