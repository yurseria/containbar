import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Provider } from "../types";
import { providerCapabilities } from "../types";

interface Props {
  containerId: string;
  containerName: string;
}

/**
 * Apple Container can only return a fresh tail snapshot, not logs since a
 * cursor. Keep the longest previous-suffix/current-prefix overlap and append
 * only the lines that appeared after it.
 */
function newLinesFromSnapshot(previous: string[], current: string[]): string[] {
  const maxOverlap = Math.min(previous.length, current.length);
  for (let overlap = maxOverlap; overlap > 0; overlap -= 1) {
    let matches = true;
    const previousStart = previous.length - overlap;
    for (let index = 0; index < overlap; index += 1) {
      if (previous[previousStart + index] !== current[index]) {
        matches = false;
        break;
      }
    }
    if (matches) return current.slice(overlap);
  }
  return current;
}

export function LogWindow({ containerId, containerName }: Props) {
  const [logs, setLogs] = useState<string[]>([]);
  const [follow, setFollow] = useState(true);
  const [lineWrap, setLineWrap] = useState(true);
  const [timestamps, setTimestamps] = useState(false);
  const [provider, setProvider] = useState<Provider | null>(null);
  const caps = providerCapabilities(provider ?? "apple");
  const bottomRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastFetchRef = useRef<number>(0);
  const appleSnapshotRef = useRef<string[]>([]);

  useEffect(() => {
    invoke<Provider>("get_provider").then(setProvider).catch(() => {});
  }, []);

  // Initial fetch + refetch when timestamps toggle changes
  useEffect(() => {
    const fetchInitial = async () => {
      try {
        const lines = await invoke<string[]>("get_container_logs", {
          id: containerId,
          tail: "200",
          timestamps,
        });
        setLogs(lines);
        appleSnapshotRef.current = lines;
        lastFetchRef.current = Math.floor(Date.now() / 1000);
      } catch {
        // ignore
      }
    };
    fetchInitial();
  }, [containerId, timestamps]);

  // Incremental polling
  useEffect(() => {
    const poll = setInterval(async () => {
      if (provider === null || lastFetchRef.current === 0) return;
      try {
        const newLines = await invoke<string[]>("get_container_logs_since", {
          id: containerId,
          since: lastFetchRef.current,
          timestamps,
        });
        if (provider === "apple") {
          const delta = newLinesFromSnapshot(appleSnapshotRef.current, newLines);
          appleSnapshotRef.current = newLines;
          if (delta.length > 0) {
            setLogs((prev) => [...prev, ...delta]);
          }
          lastFetchRef.current = Math.floor(Date.now() / 1000);
        } else if (newLines.length > 0) {
          setLogs((prev) => [...prev, ...newLines]);
          lastFetchRef.current = Math.floor(Date.now() / 1000);
        }
      } catch {
        // ignore
      }
    }, 1000);
    return () => clearInterval(poll);
  }, [containerId, provider, timestamps]);

  // Follow tail
  useEffect(() => {
    if (follow && bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: "smooth" });
    }
  }, [logs, follow]);

  // Detect manual scroll to auto-toggle follow
  useEffect(() => {
    const el = contentRef.current;
    if (!el) return;
    const handleScroll = () => {
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
      setFollow(atBottom);
    };
    el.addEventListener("scroll", handleScroll);
    return () => el.removeEventListener("scroll", handleScroll);
  }, []);

  const clearLogs = () => setLogs([]);

  return (
    <div className="log-window">
      <div className="log-window-header">
        <span className="log-window-title">{containerName}</span>
        <span className="log-window-id">{containerId}</span>
        <div className="log-window-controls">
          <button
            className={`log-toggle-btn ${lineWrap ? "active" : ""}`}
            onClick={() => setLineWrap((v) => !v)}
            title="Line Wrap"
          >
            <i className="ri-text-wrap" /> Wrap
          </button>
          {caps.logTimestamps && (
            <button
              className={`log-toggle-btn ${timestamps ? "active" : ""}`}
              onClick={() => setTimestamps((v) => !v)}
              title="Timestamps"
            >
              <i className="ri-time-line" /> Time
            </button>
          )}
          <button
            className={`log-toggle-btn ${follow ? "active" : ""}`}
            onClick={() => {
              setFollow((v) => !v);
              if (!follow && bottomRef.current) {
                bottomRef.current.scrollIntoView({ behavior: "smooth" });
              }
            }}
            title="Follow tail"
          >
            <i className="ri-arrow-down-line" /> Follow
          </button>
          <button
            className="log-toggle-btn"
            onClick={clearLogs}
            title="Clear"
          >
            <i className="ri-delete-bin-line" />
          </button>
          <span className="log-streaming-dot" />
        </div>
      </div>
      <div
        ref={contentRef}
        className={`log-window-content ${lineWrap ? "wrap" : "nowrap"}`}
      >
        {logs.map((line, i) => {
          let ts = "";
          let text = line;
          if (timestamps) {
            const match = line.match(/^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)\s?(.*)/);
            if (match) {
              ts = match[1];
              text = match[2];
            }
          }
          return (
            <div key={i} className="log-window-line">
              <span className="log-line-num">{i + 1}</span>
              {ts && <span className="log-line-ts">{ts} </span>}
              <span className="log-line-text">{text}</span>
            </div>
          );
        })}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
