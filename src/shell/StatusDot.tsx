import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Health } from "../lib/types";
import { useApp } from "../stores/app";

export function statusInfo(h: Health | null): { tone: "local" | "remote" | "relay" | "offline" | "pending"; label: string } {
  switch (h?.status) {
    case "local":
      return { tone: "local", label: "Connected (local network)" };
    case "remote":
      return { tone: "remote", label: "Connected (remote)" };
    case "relay":
      return { tone: "relay", label: "Connected through relay" };
    case "offline":
      return { tone: "offline", label: "Server offline" };
    case "starting":
      return { tone: "relay", label: "Server is starting" };
    case "updating":
      return { tone: "relay", label: "Server is updating" };
    case "noAuth":
      return { tone: "offline", label: "Signed out" };
    default:
      return { tone: "pending", label: "Checking connection" };
  }
}

function ago(ts: number | null | undefined): string {
  if (!ts) return "—";
  const s = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  return `${Math.floor(s / 3600)} hr ago`;
}

/** Sidebar connection dot (spec 07) with a details popover. */
export function StatusDot({ compact = false }: { compact?: boolean }) {
  const health = useApp((s) => s.health);
  const [open, setOpen] = useState(false);
  const btn = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  const id = useId();
  const { tone, label } = statusInfo(health);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!pop.current?.contains(e.target as Node) && !btn.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        btn.current?.focus();
      }
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const rect = open ? btn.current?.getBoundingClientRect() : undefined;
  const conn = health?.connection ?? (health?.status === "offline" ? null : undefined);

  return (
    <>
      <button
        ref={btn}
        type="button"
        className={`status-dot-btn ${compact ? "is-compact" : ""}`}
        aria-label={`Server status: ${label}`}
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen((v) => !v)}
      >
        <span className={`status-dot status-dot--${tone}`} aria-hidden="true" />
        {!compact && <span className="status-dot-btn__text truncate">{health?.serverName ?? "Server"}</span>}
      </button>
      {open &&
        rect &&
        createPortal(
          <div
            ref={pop}
            id={id}
            role="dialog"
            aria-label="Server status"
            className="status-popover"
            style={{ left: rect.left, bottom: window.innerHeight - rect.top + 8 }}
          >
            <div className="status-popover__head">
              <span className={`status-dot status-dot--${tone}`} aria-hidden="true" />
              <strong>{label}</strong>
            </div>
            <dl className="status-popover__list">
              <dt>Server</dt>
              <dd>{health?.serverName ?? "—"}</dd>
              <dt>Connection</dt>
              <dd>{conn ? conn[0].toUpperCase() + conn.slice(1) : "—"}</dd>
              <dt>Response time</dt>
              <dd className="tabular">{health?.latencyMs != null ? `${health.latencyMs} ms` : "—"}</dd>
              <dt>Server version</dt>
              <dd className="tabular">{health?.serverVersion ?? "—"}</dd>
              {health?.status === "offline" && (
                <>
                  <dt>Last seen</dt>
                  <dd>{ago(health.lastSeenAt)}</dd>
                </>
              )}
              <dt>Library scan</dt>
              <dd className="tabular">
                {health?.scan ? `${health.scan.done.toLocaleString()} of ${health.scan.total.toLocaleString()}` : "Idle"}
              </dd>
              <dt>Music files</dt>
              <dd>{health ? (health.filesAvailable ? "Available" : "Unavailable") : "—"}</dd>
            </dl>
          </div>,
          document.body,
        )}
    </>
  );
}
