import { ContextMenuHost } from "../components/ContextMenu";
import { CoverLightSource } from "./CoverLightSource";
import { LinkAskDialog } from "../components/LinkAskDialog";
import { PlaylistDialogs } from "../components/PlaylistDialogs";
import { CircleDialogs } from "../components/CircleDialogs";
import { FolderDialogs } from "./FolderMenus";
import { SavedInHost } from "../components/SavedInMenu";
import { Toaster } from "../components/Toast";
import { DownloadDialogs } from "../components/DownloadButton";
import { ShareMusicDialog } from "./ShareMusic";
import { WhatsNewDialog } from "../help/WhatsNewDialog";
import { ArchiveDialogs } from "../components/ArchiveDialogs";
import { PhysicalDialogHost } from "../components/PhysicalDialog";
import { SongCreditsHost } from "../components/SongCreditsDialog";
import {
  LazyGuideHost,
  LazyPickLookHost,
  LazyPlexEditDialogs,
  LazyShareCardHost,
  LazySmartEditorHost,
  LazyStatsDialogHost,
  LazyTagEditDialogs,
  LazyLyricsEditorHost,
  LazyWrappedHost,
  LazyWrongLyricsHost,
} from "./lazyHosts";

/** Dialogs, menus and hosts every frame carries, on a phone as well as a computer. */
export function ShellHosts() {
  return (
    <>
      <LazyGuideHost />
      <WhatsNewDialog />
      <ShareMusicDialog />
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
      <LazyLyricsEditorHost />
      <LazyWrongLyricsHost />
      <LazyShareCardHost />
      <LazyStatsDialogHost />
      <LazyWrappedHost />
      <LazyPickLookHost />
      <CoverLightSource />
    </>
  );
}
