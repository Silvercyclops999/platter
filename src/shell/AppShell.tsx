import { useEffect, useState } from "react";
import { ContextMenuHost } from "../components/ContextMenu";
import { Toaster } from "../components/Toast";
import { chrome } from "../lib/platform";
import { useUi } from "../stores/ui";
import { CommandPalette } from "./CommandPalette";
import { MainView } from "./MainView";
import { PlayerBar } from "./PlayerBar";
import { QueuePanel } from "./QueuePanel";
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

/** Sidebar | main | queue, over the player bar; custom title bar on Windows. */
export function AppShell() {
  useShortcuts();
  const queueOpen = useUi((s) => s.queueOpen);
  const width = useWindowWidth();
  // Keep the main view usable on small windows: collapse the sidebar to icons.
  const collapsed = width < 900 || (queueOpen && width < 1200);

  return (
    <div className={`app app--chrome-${chrome} ${queueOpen ? "has-queue" : ""} ${collapsed ? "sidebar-collapsed" : ""}`}>
      <TitleBar />
      <div className="app__body">
        <Sidebar collapsed={collapsed} />
        <MainView />
        {queueOpen && <QueuePanel />}
      </div>
      <PlayerBar />
      <CommandPalette />
      <ContextMenuHost />
      <Toaster />
    </div>
  );
}
