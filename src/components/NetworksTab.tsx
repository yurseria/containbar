import { useState } from "react";
import type { NetworkInfo } from "../types";
import { ContextMenu, type MenuItem } from "./ContextMenu";

interface Props {
  networks: NetworkInfo[];
  search: string;
  onRemove: (id: string) => Promise<void>;
}

export function NetworksTab({ networks, search, onRemove }: Props) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);

  const filtered = networks.filter((n) => {
    if (!search) return true;
    const q = search.toLowerCase();
    return n.name.toLowerCase().includes(q) || n.driver.toLowerCase().includes(q);
  });

  if (filtered.length === 0) {
    return <div className="empty">{search ? "No matching networks" : "No networks found"}</div>;
  }

  return (
    <div className="tab-content" onContextMenu={(e) => e.preventDefault()}>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {filtered.map((n) => (
        <div
          key={n.id}
          className={`list-item glass-surface clickable ${expanded === n.id ? "expanded" : ""}`}
          onClick={() => setExpanded(expanded === n.id ? null : n.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setMenu({
              x: e.clientX, y: e.clientY,
              items: [{ label: "Remove", danger: true, confirm: `Remove network "${n.name}"?`, onClick: () => onRemove(n.id) }],
            });
          }}
        >
          <div className="item-main">
            <div className="item-info">
              <span className="item-name">{n.name}</span>
              <span className="item-detail">
                {n.driver} / {n.scope}
              </span>
            </div>
            <div className="item-badges">
              {n.containers > 0 && (
                <span className="size-badge">
                  {n.containers} container{n.containers !== 1 ? "s" : ""}
                </span>
              )}
            </div>
          </div>
          <div className="item-meta">
            <span>{n.id}</span>
          </div>
          {expanded === n.id && (
            <div className="container-detail">
              <div className="detail-row">
                <span className="detail-label">ID</span>
                <span className="detail-value">{n.id}</span>
              </div>
              <div className="detail-row">
                <span className="detail-label">Name</span>
                <span className="detail-value">{n.name}</span>
              </div>
              <div className="detail-row">
                <span className="detail-label">Driver</span>
                <span className="detail-value">{n.driver}</span>
              </div>
              <div className="detail-row">
                <span className="detail-label">Scope</span>
                <span className="detail-value">{n.scope}</span>
              </div>
              <div className="detail-row">
                <span className="detail-label">Containers</span>
                <span className="detail-value">{n.containers}</span>
              </div>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
