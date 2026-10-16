/**
 * Rarely used dialogs load their code the first time they open, so it isn't
 * parsed at startup. After the first open the host stays mounted, so closing
 * still animates.
 */
import { lazy, Suspense, useEffect, useState } from "react";
import { useCurate } from "../stores/curate";
import { useLyricsEditor } from "../stores/lyricsEditor";
import { useWrongLyrics } from "../stores/wrongLyrics";
import { ensureLoaded, usePlexEdit } from "../stores/plexEdit";
import { useShareCard } from "../stores/shareCard";
import { useTagEdit } from "../stores/tagEdit";
import { useStatsDialog } from "../stores/stats";
import { useLooks } from "../stores/looks";

const SmartEditor = lazy(() => import("../components/SmartPlaylistEditor").then((m) => ({ default: m.SmartEditorHost })));
const LyricsEditor = lazy(() => import("./LyricsEditor").then((m) => ({ default: m.LyricsEditorHost })));
const WrongLyrics = lazy(() => import("./LyricsTrust").then((m) => ({ default: m.WrongLyricsDialog })));
const ShareCard = lazy(() => import("../components/ShareCardDialog").then((m) => ({ default: m.ShareCardHost })));
const StatsDialog = lazy(() => import("../components/StatsDialog").then((m) => ({ default: m.StatsDialogHost })));
const LookBuilder = lazy(() => import("../pages/looks/LookBuilder").then((m) => ({ default: m.LookBuilderHost })));
const PickLook = lazy(() => import("../pages/looks/LookCards").then((m) => ({ default: m.PickLookDialog })));
const TagEdit = lazy(() => import("../components/TagEditDialogs").then((m) => ({ default: m.TagEditDialogs })));
const PlexEdit = lazy(() => import("../components/PlexEditDialogs").then((m) => ({ default: m.PlexEditDialogs })));

/** True from the first time `open` is true onwards. */
function useOnceOpen(open: boolean): boolean {
  const [seen, setSeen] = useState(open);
  if (open && !seen) setSeen(true);
  return seen || open;
}

export function LazySmartEditorHost() {
  const show = useOnceOpen(useCurate((s) => !!s.editor));
  return show ? (
    <Suspense fallback={null}>
      <SmartEditor />
    </Suspense>
  ) : null;
}

export function LazyLyricsEditorHost() {
  const show = useOnceOpen(useLyricsEditor((s) => !!s.track));
  return show ? (
    <Suspense fallback={null}>
      <LyricsEditor />
    </Suspense>
  ) : null;
}

export function LazyWrongLyricsHost() {
  const show = useOnceOpen(useWrongLyrics((s) => !!s.track));
  return show ? (
    <Suspense fallback={null}>
      <WrongLyrics />
    </Suspense>
  ) : null;
}

export function LazyPlexEditDialogs() {
  // Menus are built on demand, so what's editable is known before the first one opens.
  useEffect(() => ensureLoaded(), []);
  const show = useOnceOpen(usePlexEdit((s) => !!s.dialog));
  return show ? (
    <Suspense fallback={null}>
      <PlexEdit />
    </Suspense>
  ) : null;
}

export function LazyTagEditDialogs() {
  const show = useOnceOpen(useTagEdit((s) => !!(s.editor || s.check || s.fix || s.explicit)));
  return show ? (
    <Suspense fallback={null}>
      <TagEdit />
    </Suspense>
  ) : null;
}

export function LazyShareCardHost() {
  const show = useOnceOpen(useShareCard((s) => !!s.subject));
  return show ? (
    <Suspense fallback={null}>
      <ShareCard />
    </Suspense>
  ) : null;
}

export function LazyStatsDialogHost() {
  const show = useOnceOpen(useStatsDialog((s) => !!s.subject));
  return show ? (
    <Suspense fallback={null}>
      <StatsDialog />
    </Suspense>
  ) : null;
}

export function LazyLookBuilderHost() {
  const show = useOnceOpen(useLooks((s) => s.builder !== null));
  return show ? (
    <Suspense fallback={null}>
      <LookBuilder />
    </Suspense>
  ) : null;
}

export function LazyPickLookHost() {
  const show = useOnceOpen(useLooks((s) => s.pick));
  return show ? (
    <Suspense fallback={null}>
      <PickLook />
    </Suspense>
  ) : null;
}
