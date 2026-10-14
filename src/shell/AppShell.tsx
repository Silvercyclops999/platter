import { PanelRightOpen } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { ContextMenuHost } from "../components/ContextMenu";
import { Toaster } from "../components/Toast";
import { Tooltip } from "../components/Tooltip";
import { chrome } from "../lib/platform";
import { useUi } from "../stores/ui";
import { CommandPalette } from "./CommandPalette";
import {
  RAIL_WIDTH,
  RIGHT_MAX,
  RIGHT_MIN,
  SIDEBAR_MAX,
  dragRight,
  dragSidebar,
  keyRight,
  keySidebar,
  rightRoom,
  sidebarRoom,
  solveLayout,
  type SidebarDrag,
} from "./layout";
import { MainView } from "./MainView";
import { PlayerBar } from "./PlayerBar";
import { QueuePanel } from "./QueuePanel";
import { ResizeHandle } from "./ResizeHandle";
import { Sidebar } from "./Sidebar";
import { TitleBar } from "./TitleBar";
import { useShortcuts } from "./useShortcuts";

function useWindowWidth() {
  const [w, setW] = useState(() => (typeof window !== "undefined" ? window.innerWidth : 1280));
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

/** Sidebar | main | right panel, over the player bar; custom title bar on Windows. */
export function AppShell() {
  useShortcuts();
  const queueOpen = useUi((s) => s.queueOpen);
  const sidebarWidth = useUi((s) => s.sidebarWidth);
  const sidebarCollapsed = useUi((s) => s.sidebarCollapsed);
  const rightWidth = useUi((s) => s.rightWidth);
  const width = useWindowWidth();
  const shell = useRef<HTMLDivElement>(null);
  const layout = solveLayout({ windowWidth: width, sidebarWidth, rightWidth, rightOpen: queueOpen, sidebarCollapsed });
  const rail = layout.sidebarMode === "rail";

  // While dragging, widths go straight to CSS variables on the shell; the
  // store (and React) only hear about it on release, or when the sidebar
  // snaps between the rail and full mode.
  const setVar = (name: string, px: number) => shell.current?.style.setProperty(name, `${px}px`);

  const sidebarDrag = () => {
    const start: SidebarDrag = { width: layout.sidebar, rail };
    const room = sidebarRoom(width, layout.right);
    let cur = start;
    return {
      move: (dx: number) => {
        const next = dragSidebar(start, dx, room);
        if (next.rail !== cur.rail) useUi.getState().setSidebar(next.rail ? { collapsed: true } : { collapsed: false, width: next.width });
        cur = next;
        setVar("--sidebar-w", next.width);
        return next.rail ? RAIL_WIDTH : next.width;
      },
      end: () => useUi.getState().setSidebar(cur.rail ? { collapsed: true } : { collapsed: false, width: cur.width }),
    };
  };

  const rightDrag = () => {
    const start = layout.right;
    const room = rightRoom(width, layout.sidebar);
    let cur = start;
    return {
      move: (dx: number) => {
        cur = dragRight(start, dx, room);
        setVar("--right-w", cur);
        return cur;
      },
      end: () => useUi.getState().setRightWidth(cur),
    };
  };

  const style = { "--sidebar-w": `${layout.sidebar}px`, "--right-w": `${layout.right}px` } as CSSProperties;

  return (
    <div ref={shell} className={`app app--chrome-${chrome} ${queueOpen ? "has-queue" : ""} ${rail ? "sidebar-collapsed" : ""}`} style={style}>
      <TitleBar />
      <div className="app__body">
        <Sidebar collapsed={rail} />
        {/* An auto-collapsed rail has no room to open into, so no handle. */}
        {!layout.sidebarAuto && (
          <ResizeHandle
            className="resize-handle--sidebar"
            label="Resize sidebar"
            value={rail ? RAIL_WIDTH : layout.sidebar}
            min={RAIL_WIDTH}
            max={SIDEBAR_MAX}
            shell={shell}
            begin={sidebarDrag}
            onKey={(key) => {
              const next = keySidebar({ width: layout.sidebar, rail }, key, sidebarRoom(width, layout.right));
              if (!next) return false;
              useUi.getState().setSidebar(next.rail ? { collapsed: true } : { collapsed: false, width: next.width });
              return true;
            }}
          />
        )}
        <MainView />
        {queueOpen ? (
          <>
            <ResizeHandle
              className="resize-handle--right"
              label="Resize queue"
              value={layout.right}
              min={RIGHT_MIN}
              max={RIGHT_MAX}
              shell={shell}
              begin={rightDrag}
              onKey={(key) => {
                const next = keyRight(layout.right, key, rightRoom(width, layout.sidebar));
                if (next === null) return false;
                useUi.getState().setRightWidth(next);
                return true;
              }}
            />
            <QueuePanel />
          </>
        ) : (
          <div className="right-strip">
            <Tooltip label="Open queue">
              <button type="button" className="right-strip__btn" aria-label="Open queue" onClick={() => useUi.getState().setQueueOpen(true)}>
                <PanelRightOpen size={16} aria-hidden="true" />
              </button>
            </Tooltip>
          </div>
        )}
      </div>
      <PlayerBar />
      <CommandPalette />
      <ContextMenuHost />
      <Toaster />
    </div>
  );
}
