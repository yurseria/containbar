import { useState } from "react";
import type { VolumeInfo } from "../types";
import { ContextMenu, type MenuItem } from "./ContextMenu";

interface Props {
  volumes: VolumeInfo[];
  search: string;
  onRemove: (name: string) => Promise<void>;
}

export function VolumesTab({ volumes, search, onRemove }: Props) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);

  const filtered = volumes.filter((v) => {
    if (!search) return true;
    const q = search.toLowerCase();
    return v.name.toLowerCase().includes(q) || v.driver.toLowerCase().includes(q);
  });

  if (filtered.length === 0) {
    return <div className="empty">{search ? "No matching volumes" : "No volumes found"}</div>;
  }

  return (
    <div className="tab-content" onContextMenu={(e) => e.preventDefault()}>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {filtered.map((v) => (
        <div
          key={v.name}
          className={`list-item glass-surface clickable ${expanded === v.name ? "expanded" : ""}`}
          onClick={() => setExpanded(expanded === v.name ? null : v.name)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setMenu({
              x: e.clientX, y: e.clientY,
              items: [{ label: "Remove", danger: true, confirm: `Remove volume "${v.name}"?`, onClick: () => onRemove(v.name) }],
            });
          }}
        >
          <div className="item-main">
            <div className="item-info">
              <span className="item-name">{v.name}</span>
              <span className="item-detail">{v.driver}</span>
            </div>
          </div>
          <div className="item-meta">
            <span className="mountpoint" title={v.mountpoint}>
              {v.mountpoint}
            </span>
          </div>
          {expanded === v.name && (
            <div className="container-detail">
              <div className="detail-row">
                <span className="detail-label">Name</span>
                <span className="detail-value">{v.name}</span>
              </div>
              <div className="detail-row">
                <span className="detail-label">Driver</span>
                <span className="detail-value">{v.driver}</span>
              </div>
              <div className="detail-row">
                <span className="detail-label">Mount</span>
                <span className="detail-value">{v.mountpoint}</span>
              </div>
              {Object.keys(v.labels).length > 0 && (
                <div className="detail-row">
                  <span className="detail-label">Labels</span>
                  <span className="detail-value">{Object.entries(v.labels).map(([k, val]) => `${k}=${val}`).join(", ")}</span>
                </div>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
