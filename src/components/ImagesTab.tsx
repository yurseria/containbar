import { useState } from "react";
import type { ImageInfo } from "../types";
import { ContextMenu, type MenuItem } from "./ContextMenu";

interface Props {
  images: ImageInfo[];
  search: string;
  onRemove: (id: string) => Promise<void>;
  onCreateContainer?: (image: string) => void;
}

function formatSize(bytes: number): string {
  if (bytes <= 0) return "N/A";
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatDate(ts: number): string {
  if (ts <= 0) return "N/A";
  return new Date(ts * 1000).toLocaleDateString();
}

function formatDateTime(ts: number): string {
  if (ts <= 0) return "N/A";
  return new Date(ts * 1000).toLocaleString();
}

function imageDisplayName(image: ImageInfo): string {
  return image.repo_tags[0] || image.id;
}

export function ImagesTab({ images, search, onRemove, onCreateContainer }: Props) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);

  const filtered = images
    .filter((img) => {
      if (!search) return true;
      const q = search.toLowerCase();
      return img.repo_tags.some((t) => t.toLowerCase().includes(q)) || img.id.toLowerCase().includes(q);
    })
    .sort((a, b) => imageDisplayName(a).localeCompare(imageDisplayName(b), undefined, {
      numeric: true,
      sensitivity: "base",
    }));

  if (filtered.length === 0) {
    return <div className="empty">{search ? "No matching images" : "No images found"}</div>;
  }

  return (
    <div className="tab-content" onContextMenu={(e) => e.preventDefault()}>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {filtered.map((img) => (
        <div
          key={img.id}
          className={`list-item glass-surface clickable ${expanded === img.id ? "expanded" : ""}`}
          onClick={() => setExpanded(expanded === img.id ? null : img.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setMenu({
              x: e.clientX, y: e.clientY,
              items: [
                ...(onCreateContainer ? [{ label: "Create Container", onClick: () => onCreateContainer(img.repo_tags[0] || img.id) }] : []),
                { label: "Remove", danger: true, confirm: `Remove image "${img.repo_tags[0] || img.id}"?`, onClick: () => onRemove(img.id) },
              ],
            });
          }}
        >
          <div className="item-main">
            <div className="item-info">
              <span className="item-name">
                {img.repo_tags[0] || `<none>:${img.id}`}
              </span>
              <span className="item-detail">{img.id}</span>
            </div>
            <div className="item-badges">
              <span className="size-badge">{formatSize(img.size)}</span>
            </div>
          </div>
          <div className="item-meta">
            <span>Created: {formatDate(img.created)}</span>
            {img.repo_tags.length > 1 && (
              <span>+{img.repo_tags.length - 1} tags</span>
            )}
          </div>
          {expanded === img.id && (
            <div className="container-detail">
              <div className="detail-row">
                <span className="detail-label">ID</span>
                <span className="detail-value">{img.id}</span>
              </div>
              <div className="detail-row">
                <span className="detail-label">Size</span>
                <span className="detail-value">{formatSize(img.size)}</span>
              </div>
              <div className="detail-row">
                <span className="detail-label">Created</span>
                <span className="detail-value">{formatDateTime(img.created)}</span>
              </div>
              {img.repo_tags.length > 0 && (
                <div className="detail-row">
                  <span className="detail-label">Tags</span>
                  <span className="detail-value">{img.repo_tags.join(", ")}</span>
                </div>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
