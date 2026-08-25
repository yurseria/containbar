import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ContainerGroup } from "../types";
import { getSettings } from "./Settings";
import { ContextMenu, type MenuItem } from "./ContextMenu";

interface Props {
  groups: ContainerGroup[];
  search: string;
  onStart: (id: string) => Promise<void>;
  onStop: (id: string) => Promise<void>;
  onRestart: (id: string) => Promise<void>;
  onStartGroup: (ids: string[]) => Promise<void>;
  onStopGroup: (ids: string[]) => Promise<void>;
  onRemove: (id: string, force?: boolean) => Promise<void>;
  getEnv: (id: string) => Promise<string[]>;
}

function displayContainerId(id: string) {
  return /^[a-f0-9]{24,}$/i.test(id) ? id.slice(0, 12) : id;
}

export function ContainersTab({ groups, search, onStart, onStop, onRestart, onStartGroup, onStopGroup, onRemove, getEnv }: Props) {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [expanded, setExpanded] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [groupLoading, setGroupLoading] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [envVars, setEnvVars] = useState<Record<string, string[]>>({});

  useEffect(() => {
    if (expanded && !envVars[expanded]) {
      getEnv(expanded).then((env) => {
        setEnvVars((prev) => ({ ...prev, [expanded]: env }));
      });
    }
  }, [expanded, envVars, getEnv]);

  const toggle = (name: string) =>
    setCollapsed((prev) => ({ ...prev, [name]: !prev[name] }));

  const handleAction = async (id: string, action: (id: string) => Promise<void>) => {
    setActionLoading(id);
    try {
      await action(id);
    } finally {
      setActionLoading(null);
    }
  };

  const handleGroupAction = async (
    group: ContainerGroup,
    action: (ids: string[]) => Promise<void>,
  ) => {
    setGroupLoading(group.name);
    try {
      await action(group.containers.map((container) => container.id));
    } finally {
      setGroupLoading(null);
    }
  };

  const handleGroupRemove = async (group: ContainerGroup) => {
    setGroupLoading(group.name);
    try {
      for (const c of group.containers) {
        await onRemove(c.id, c.state === "running");
      }
    } finally {
      setGroupLoading(null);
    }
  };

  const openLogs = (id: string, name: string) => {
    invoke("open_log_window", { containerId: id, containerName: name });
  };

  const openTerminal = (id: string, name: string) => {
    const s = getSettings();
    invoke("open_terminal", {
      containerId: id,
      containerName: name,
      shell: s.shell,
      terminalOverride: s.terminal,
    });
  };

  const openFiles = (id: string, name: string) => {
    invoke("open_file_explorer_window", { containerId: id, containerName: name });
  };

  const filteredGroups = groups
    .map((group) => ({
      ...group,
      containers: group.containers.filter((c) => {
        if (!search) return true;
        const q = search.toLowerCase();
        return (
          (c.names[0] || c.id).toLowerCase().includes(q) ||
          c.image.toLowerCase().includes(q) ||
          group.name.toLowerCase().includes(q)
        );
      }),
    }))
    .filter((g) => g.containers.length > 0);

  if (filteredGroups.length === 0) {
    return <div className="empty">{search ? "No matching containers" : "No containers found"}</div>;
  }

  return (
    <div className="tab-content" onContextMenu={(e) => e.preventDefault()}>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {filteredGroups.map((group) => {
        const hasRunning = group.containers.some((c) => c.state === "running");
        const hasStopped = group.containers.some((c) => c.state !== "running");
        return (
          <div key={group.name} className="group">
            <div
              className="group-header"
              onClick={() => toggle(group.name)}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setMenu({
                  x: e.clientX, y: e.clientY,
                  items: [
                    ...(group.name !== "Standalone" && hasStopped ? [{ label: "Start All", onClick: () => handleGroupAction(group, onStartGroup) }] : []),
                    ...(group.name !== "Standalone" && hasRunning ? [{ label: "Stop All", onClick: () => handleGroupAction(group, onStopGroup) }] : []),
                    { label: "Remove All", danger: true, confirm: `Remove all containers in "${group.name}"?`, onClick: () => handleGroupRemove(group) },
                  ],
                });
              }}
            >
              <i className={`chevron ${collapsed[group.name] ? "ri-arrow-right-s-line" : "ri-arrow-down-s-line"}`} />
              <span className="group-name">{group.name}</span>
              <span className="group-count">{group.containers.length}</span>
              {group.name !== "Standalone" && (
                <div className="group-actions" onClick={(e) => e.stopPropagation()}>
                  {hasStopped && (
                    <button className="action-btn start" title="Start All" disabled={groupLoading === group.name} onClick={() => handleGroupAction(group, onStartGroup)}>
                      <i className="ri-play-fill" />
                    </button>
                  )}
                  {hasRunning && (
                    <button className="action-btn stop" title="Stop All" disabled={groupLoading === group.name} onClick={() => handleGroupAction(group, onStopGroup)}>
                      <i className="ri-stop-fill" />
                    </button>
                  )}
                </div>
              )}
            </div>
            {!collapsed[group.name] && (
              <div className="group-items">
                {group.containers.map((c) => {
                  const name = c.names[0] || c.id;
                  return (
                    <div
                      key={c.id}
                      className={`container-item state-${c.state} ${expanded === c.id ? "expanded" : ""}`}
                      onClick={() => setExpanded(expanded === c.id ? null : c.id)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        const items: MenuItem[] = [];
                        if (c.state === "running") {
                          items.push({ label: "Stop & Remove", danger: true, confirm: `Stop and remove "${name}"?`, onClick: () => onRemove(c.id, true) });
                        } else {
                          items.push({ label: "Remove", danger: true, confirm: `Remove "${name}"?`, onClick: () => onRemove(c.id) });
                        }
                        setMenu({ x: e.clientX, y: e.clientY, items });
                      }}
                    >
                      <div className="container-main">
                        <span className={`status-dot ${c.state}`} />
                        <div className="container-info">
                          <span className="container-name">{name}</span>
                          <span className="container-image">{c.image}</span>
                        </div>
                        <div className="container-actions" onClick={(e) => e.stopPropagation()}>
                          {c.state === "running" ? (
                            <>
                              <button className="action-btn stop" title="Stop" disabled={actionLoading === c.id} onClick={() => handleAction(c.id, onStop)}>
                                <i className="ri-stop-fill" />
                              </button>
                              <button className="action-btn restart" title="Restart" disabled={actionLoading === c.id} onClick={() => handleAction(c.id, onRestart)}>
                                <i className="ri-restart-line" />
                              </button>
                              <button className="action-btn terminal" title="Terminal" onClick={() => openTerminal(c.id, name)}>
                                <i className="ri-terminal-box-line" />
                              </button>
                            </>
                          ) : (
                            <button className="action-btn start" title="Start" disabled={actionLoading === c.id} onClick={() => handleAction(c.id, onStart)}>
                              <i className="ri-play-fill" />
                            </button>
                          )}
                          <button className="action-btn logs" title="Logs" onClick={() => openLogs(c.id, name)}>
                            <i className="ri-file-text-line" />
                          </button>
                          <button className="action-btn finder" title="Browse Files" onClick={() => openFiles(c.id, name)}>
                            <i className="ri-folder-open-line" />
                          </button>
                        </div>
                      </div>
                      <div className="container-meta">
                        <span className="container-status">{c.status}</span>
                        {[...new Set(
                          c.ports
                            .filter((p) => p.public_port)
                            .map((p) => `${p.public_port}:${p.private_port}`)
                        )].map((key) => {
                          const [publicPort, privatePort] = key.split(":").map(Number);
                          return (
                            <span key={key} className="port-badge">
                              {publicPort}:{privatePort}
                            </span>
                          );
                        })}
                      </div>
                      {expanded === c.id && (
                        <div className="container-detail">
                          <div className="detail-row">
                            <span className="detail-label">ID</span>
                            <span className="detail-value">{displayContainerId(c.id)}</span>
                          </div>
                          <div className="detail-row">
                            <span className="detail-label">Image</span>
                            <span className="detail-value">{c.image}</span>
                          </div>
                          <div className="detail-row">
                            <span className="detail-label">State</span>
                            <span className="detail-value">{c.state}</span>
                          </div>
                          <div className="detail-row">
                            <span className="detail-label">Status</span>
                            <span className="detail-value">{c.status}</span>
                          </div>
                          <div className="detail-row">
                            <span className="detail-label">Created</span>
                            <span className="detail-value">{new Date(c.created * 1000).toLocaleString()}</span>
                          </div>
                          {envVars[c.id] && envVars[c.id].length > 0 && (
                            <div className="detail-env">
                              <span className="detail-label">Env</span>
                              <div className="env-list">
                                {envVars[c.id].map((e, i) => (
                                  <span key={i} className="env-item">{e}</span>
                                ))}
                              </div>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
