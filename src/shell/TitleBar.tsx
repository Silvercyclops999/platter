import { Copy, Minus, Square, X } from "lucide-react";
import { useEffect, useState } from "react";
import { chrome } from "../lib/platform";

type AppWindow = {
  minimize: () => Promise<void>;
  toggleMaximize: () => Promise<void>;
  close: () => Promise<void>;
  isMaximized: () => Promise<boolean>;
  onResized: (cb: () => void) => Promise<() => void>;
};

/**
 * Windows: custom title bar (window is undecorated) with a drag region and
 * min / max / close. macOS: an empty drag strip that leaves room for the
 * overlay traffic lights. Plain browser: nothing.
 */
export function TitleBar() {
  const [win, setWin] = useState<AppWindow | null>(null);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (chrome !== "windows") return;
    let un: (() => void) | undefined;
    let alive = true;
    import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
      const w = getCurrentWindow() as unknown as AppWindow;
      if (!alive) return;
      setWin(w);
      setMaximized(await w.isMaximized());
      un = await w.onResized(async () => setMaximized(await w.isMaximized()));
    });
    return () => {
      alive = false;
      un?.();
    };
  }, []);

  if (chrome === "none") return null;
  if (chrome === "mac") return <div className="titlebar titlebar--mac" data-tauri-drag-region />;

  return (
    <div className="titlebar titlebar--windows" data-tauri-drag-region>
      <span className="titlebar__name" data-tauri-drag-region>
        Platter
      </span>
      <div className="titlebar__controls">
        <button type="button" className="titlebar__btn" aria-label="Minimize" onClick={() => win?.minimize()}>
          <Minus size={16} strokeWidth={1.5} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="titlebar__btn"
          aria-label={maximized ? "Restore" : "Maximize"}
          onClick={() => win?.toggleMaximize()}
        >
          {maximized ? <Copy size={13} strokeWidth={1.5} aria-hidden="true" /> : <Square size={13} strokeWidth={1.5} aria-hidden="true" />}
        </button>
        <button type="button" className="titlebar__btn titlebar__btn--close" aria-label="Close" onClick={() => win?.close()}>
          <X size={16} strokeWidth={1.5} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}
