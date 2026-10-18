import { PanelRightOpen } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { ContextMenuHost } from "../components/ContextMenu";
import { DragAnnouncer } from "../components/DragAnnouncer";
import { LinkAskDialog } from "../components/LinkAskDialog";
import { PlaylistDialogs } from "../components/PlaylistDialogs";
import { CircleDialogs } from "../components/CircleDialogs";
import { FolderDialogs } from "./FolderMenus";
import { SavedInHost } from "../components/SavedInMenu";
import { Toaster } from "../components/Toast";
import { Tooltip } from "../components/Tooltip";
import { chrome } from "../lib/platform";
import { useCurrentEntry } from "../stores/router";
import { useLayout } from "../stores/theme";
import { besideView, rightView as slotView, useUi } from "../stores/ui";
import { useFullWindow } from "../stores/fullWindow";
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
import { FullWindow } from "./FullWindow";
import { startLyricsTrust, useLyricsAutoShow } from "./lyrics";
import { startTranscribe } from "../stores/transcribe";
import { startDownloads } from "../stores/downloads";
import { DownloadDialogs } from "../components/DownloadButton";
import { loadOwnLyricsIds } from "../stores/lyricsEditor";
import { MainView } from "./MainView";
import { NowPlayingPanel, useOpenNowPlayingOnStart } from "./NowPlayingPanel";
import { OutputPanel } from "./OutputPanel";
import { PlayerBar } from "./PlayerBar";
import { RemoteStrip } from "./RemoteStrip";
import { ListenStrip } from "./ListenStrip";
import { ShareMusicDialog } from "./ShareMusic";
import { FriendActivityPanel } from "./FriendActivityPanel";
import { startSocial } from "../stores/social";
import { startCircle } from "../stores/circle";
import { startSharing } from "../stores/sharing";
import { afterFirstPaint } from "../lib/idle";
import { startCurate } from "../stores/curate";
import { DeckActions } from "./DeckActions";
import { DeckPairPrompt } from "./DeckPairPrompt";
import { backend } from "../lib/backend";
import { NewLookOffer } from "./NewLookOffer";
import { LazyLookBuilderHost, LazyLyricsEditorHost, LazyPickLookHost, LazyPlexEditDialogs, LazyShareCardHost, LazySmartEditorHost, LazyStatsDialogHost, LazyTagEditDialogs } from "./lazyHosts";
import { LazyGuideHost, LazyHelpPanel, LazyWrappedHost, LazyWrongLyricsHost } from "./lazyHosts";
import { LookDrop } from "./LookDrop";
import { QueuePanel } from "./QueuePanel";
import { WhatsNewDialog } from "../help/WhatsNewDialog";
import { ArchiveDialogs } from "../components/ArchiveDialogs";
import { PhysicalDialogHost } from "../components/PhysicalDialog";
import { SongCreditsHost } from "../components/SongCreditsDialog";
import { ResizeHandle } from "./ResizeHandle";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";
import { useShortcuts } from "./useShortcuts";
import { connectMenuBar } from "./menuBar";
import { ShortcutsHelp } from "./ShortcutsHelp";
import { PowerHints } from "./PowerHints";
import { skipToMain, useShellTabOrder } from "./focusOrder";

function useWindowWidth() {
  const [w, setW] = useState(() => (typeof window !== "undefined" ? window.innerWidth : 1280));
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

/**
 * The layout's "Sidebar collapsed" choice: applied when it changes (a look
 * or layout was picked), so the sidebar's own state from last time stays
 * otherwise.
 */
function useSidebarDefault(collapsed: boolean) {
  const prev = useRef(collapsed);
  useEffect(() => {
    if (prev.current === collapsed) return;
    prev.current = collapsed;
    useUi.getState().setSidebar({ collapsed });
  }, [collapsed]);
}

/** Top bar over sidebar | main | right panel, over the player bar. */
export function AppShell() {
  useShortcuts();
  useEffect(() => connectMenuBar(), []);
  useOpenNowPlayingOnStart();
  useLyricsAutoShow();
  useEffect(() => void loadOwnLyricsIds(), []);
  // Not needed for the first screen: wait until it has painted.
  useEffect(() => afterFirstPaint(() => void startTranscribe()), []);
  useEffect(() => afterFirstPaint(() => void startLyricsTrust()), []);
  useEffect(() => void startDownloads(), []);
  // Once, on first connect: Plex's library settings, if they'd fight the tags.
  useEffect(
    () =>
      afterFirstPaint(() => {
        if (backend.kind === "tauri") void import("../pages/TagEditorSettings").then((m) => m.firstConnectLibraryCheck());
      }),
    [],
  );
  useEffect(() => afterFirstPaint(() => void startSocial().then(() => startSharing())), []);
  useEffect(() => afterFirstPaint(() => void startCircle()), []);
  useEffect(() => void startCurate(), []);
  const queueOpen = useUi((s) => s.queueOpen);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen);
  const outputOpen = useUi((s) => s.outputOpen);
  const activityOpen = useUi((s) => s.activityOpen);
  const helpOpen = useUi((s) => s.helpOpen);
  // The queue, the now-playing panel, the output picker and friend activity share the right slot, one at a time.
  const rightView = slotView({ queueOpen, nowPlayingOpen, outputOpen, activityOpen, helpOpen });
  // Any of them but the now-playing panel sits beside the expanded view.
  const beside = besideView({ queueOpen, nowPlayingOpen, outputOpen, activityOpen, helpOpen }, useFullWindow((s) => s.open)) !== null;
  const rightOpen = rightView !== null;
  // One panel replacing the other rises in place of sliding in from the edge.
  const [slot, setSlot] = useState<{ view: typeof rightView; swap: boolean }>({ view: rightView, swap: false });
  if (slot.view !== rightView) setSlot({ view: rightView, swap: slot.view !== null && rightView !== null });
  const sidebarWidth = useUi((s) => s.sidebarWidth);
  const sidebarCollapsed = useUi((s) => s.sidebarCollapsed);
  const rightWidth = useUi((s) => s.rightWidth);
  const width = useWindowWidth();
  const shell = useRef<HTMLDivElement>(null);
  useShellTabOrder(shell);
  const layout = solveLayout({ windowWidth: width, sidebarWidth, rightWidth, rightOpen, sidebarCollapsed });
  const rail = layout.sidebarMode === "rail";
  // Wide library mode: the library page spans from the left edge, so no sidebar.
  const wideLibrary = useCurrentEntry().route.name === "library";

  // While dragging, widths go straight to CSS variables on the shell; the
  // store (and React) only hear about it on release, or when the sidebar
  // snaps between the rail and full mode.
  const setVar = (name: string, px: number) => shell.current?.style.setProperty(name, `${px}px`);

  // A sidebar on the right or a panel on the left drags the other way.
  const look = useLayout();
  const sidebarFlip = look.sidebarSide === "right" ? -1 : 1;
  const rightFlip = look.nowPlaying === "left" ? -1 : 1;
  const flipKey = (key: string, flip: number) => (flip > 0 ? key : key === "ArrowLeft" ? "ArrowRight" : key === "ArrowRight" ? "ArrowLeft" : key);
  useSidebarDefault(look.sidebarCollapsed);

  const sidebarDrag = () => {
    const start: SidebarDrag = { width: layout.sidebar, rail };
    const room = sidebarRoom(width, layout.right);
    let cur = start;
    return {
      move: (dx: number) => {
        const next = dragSidebar(start, dx * sidebarFlip, room);
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
        cur = dragRight(start, dx * rightFlip, room);
        setVar("--right-w", cur);
        return cur;
      },
      end: () => useUi.getState().setRightWidth(cur),
    };
  };

  const style = { "--sidebar-w": `${layout.sidebar}px`, "--right-w": `${layout.right}px` } as CSSProperties;

  return (
    <div ref={shell} className={`app app--chrome-${chrome} ${queueOpen ? "has-queue" : ""} ${nowPlayingOpen ? "has-now-playing" : ""} ${slot.swap ? "right-swap" : ""} ${beside ? "fw-beside" : ""} ${rail ? "sidebar-collapsed" : ""}`} style={style}>
      <a
        href="#main"
        className="skip-link"
        onClick={(e) => {
          e.preventDefault();
          skipToMain();
        }}
      >
        Skip to main content
      </a>
      <TopBar />
      <div className="app__body">
        {!wideLibrary && <Sidebar collapsed={rail} auto={layout.sidebarAuto} />}
        {/* An auto-collapsed rail has no room to open into, so no handle. */}
        {!layout.sidebarAuto && !wideLibrary && (
          <ResizeHandle
            className="resize-handle--sidebar"
            label="Resize sidebar"
            value={rail ? RAIL_WIDTH : layout.sidebar}
            min={RAIL_WIDTH}
            max={SIDEBAR_MAX}
            shell={shell}
            begin={sidebarDrag}
            onKey={(key) => {
              const next = keySidebar({ width: layout.sidebar, rail }, flipKey(key, sidebarFlip), sidebarRoom(width, layout.right));
              if (!next) return false;
              useUi.getState().setSidebar(next.rail ? { collapsed: true } : { collapsed: false, width: next.width });
              return true;
            }}
          />
        )}
        <MainView />
        {rightOpen ? (
          <>
            <ResizeHandle
              className="resize-handle--right"
              label={helpOpen ? "Resize help" : outputOpen ? "Resize device picker" : activityOpen ? "Resize listening activity" : queueOpen ? "Resize queue" : "Resize now playing view"}
              value={layout.right}
              min={RIGHT_MIN}
              max={RIGHT_MAX}
              shell={shell}
              begin={rightDrag}
              onKey={(key) => {
                const next = keyRight(layout.right, flipKey(key, rightFlip), rightRoom(width, layout.sidebar));
                if (next === null) return false;
                useUi.getState().setRightWidth(next);
                return true;
              }}
            />
            {helpOpen ? <LazyHelpPanel /> : outputOpen ? <OutputPanel /> : activityOpen ? <FriendActivityPanel /> : queueOpen ? <QueuePanel /> : <NowPlayingPanel />}
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
      <RemoteStrip />
      <ListenStrip />
      <PlayerBar />
      <NewLookOffer />
      <CommandPalette />
      <LazyGuideHost />
      <WhatsNewDialog />
      <ShareMusicDialog />
      <ShortcutsHelp />
      <PowerHints />
      <ContextMenuHost />
      <SavedInHost />
      <Toaster />
      <PlaylistDialogs />
      <LinkAskDialog />
      <CircleDialogs />
      <ArchiveDialogs />
      <PhysicalDialogHost />
      <SongCreditsHost />
      <LazyPlexEditDialogs />
      <LazyTagEditDialogs />
      <LazySmartEditorHost />
      <FolderDialogs />
      <DownloadDialogs />
      <DragAnnouncer />
      <LazyLyricsEditorHost />
      <LazyWrongLyricsHost />
      <FullWindow />
      <LazyShareCardHost />
      <DeckActions />
      <DeckPairPrompt />
      <LazyStatsDialogHost />
      <LazyWrappedHost />
      <LazyLookBuilderHost />
      <LazyPickLookHost />
      <LookDrop />
    </div>
  );
}
