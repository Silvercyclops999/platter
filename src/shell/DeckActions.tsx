/**
 * What a Stream Deck key or an automation command asks the window for
 * (`deck://action`): Listen along starts a session or shows the open
 * invite, Request album opens the usual Lidarr request dialog (its code
 * loads the first time), a page opens, a note says why a command link
 * didn't run, or a key opens the playlist picker, Listen on, the lyrics,
 * the expanded view or the queue. The window tells the keys whether the
 * lyrics show (`deck_ui_report`).
 */
import { lazy, Suspense, useEffect, useState } from "react";
import { Dialog } from "../components/Dialog";
import { PlaylistPicker } from "../components/PlaylistPicker";
import { backend } from "../lib/backend";
import type { DeckAction, LidarrRequestInput } from "../lib/types";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { loadPlayers, moveTo, useRemote } from "../stores/remote";
import { nav } from "../stores/router";
import { useUi } from "../stores/ui";
import { openInvite, startSession, useSocial } from "../stores/social";
import { toast } from "../stores/toasts";

const RequestDialog = lazy(() => import("../pages/LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

/** Carry out one Stream Deck action that needs the window. */
export function runDeckListen(): Promise<void> {
  const role = useSocial.getState().listen.role;
  if (role === "host") return openInvite();
  if (role === "guest") {
    toast.info("You're listening along with someone. Leave that session to host your own");
    return Promise.resolve();
  }
  return startSession();
}

/** Listen on from a key: the list, or straight to a device that needs the window (a password). */
export async function runDeckListenOn(device?: string): Promise<void> {
  useUi.getState().setOutputOpen(true);
  if (!device) return;
  await loadPlayers();
  const p = useRemote.getState().players?.find((x) => x.id === device);
  if (p) await moveTo(p);
}

/** Whether the lyrics show, for the Lyrics key. */
const lyricsShowing = (s: { open: boolean; mode: string }) => s.open && s.mode === "lyrics";

export function DeckActions() {
  const [input, setInput] = useState<LidarrRequestInput | null>(null);
  const [open, setOpen] = useState(false);
  const [pickFor, setPickFor] = useState<string[] | null>(null);
  useEffect(() => {
    let alive = true;
    const un = backend.listen("deck://action", (a: DeckAction) => {
      if (!alive) return;
      if (a.kind === "listenAlong") void runDeckListen();
      else if (a.kind === "open") nav.go(a.route);
      else if (a.kind === "notice") toast.info(a.message);
      else if (a.kind === "addToPlaylist") setPickFor(a.trackIds);
      else if (a.kind === "listenOn") void runDeckListenOn(a.device);
      else if (a.kind === "lyrics") {
        if (a.open) fullWindow.open("lyrics");
        else if (lyricsShowing(useFullWindow.getState())) void fullWindow.close();
      } else if (a.kind === "nowPlaying") fullWindow.open();
      else if (a.kind === "queue") useUi.getState().setQueueOpen(true);
      else {
        setInput(a.input);
        setOpen(true);
      }
    });
    // Tell the keys when the lyrics open or close.
    let last = lyricsShowing(useFullWindow.getState());
    const report = (lyrics: boolean) => void backend.deckUiReport({ lyrics }).catch(() => {});
    report(last);
    const unsub = useFullWindow.subscribe((s) => {
      const now = lyricsShowing(s);
      if (now !== last) report((last = now));
    });
    return () => {
      alive = false;
      unsub();
      void un.then((f) => f());
    };
  }, []);
  const picker = (
    <Dialog open={!!pickFor} onClose={() => setPickFor(null)} label="Add to playlist" placement="top" className="deck-picker">
      {pickFor && <PlaylistPicker songs={() => pickFor} ctx={{ close: () => setPickFor(null), back: () => setPickFor(null) }} />}
    </Dialog>
  );
  if (!input) return picker;
  return (
    <Suspense fallback={null}>
      {picker}
      <RequestDialog
        open={open}
        input={input}
        onClose={() => setOpen(false)}
        onDone={() => {
          setOpen(false);
          toast.info(`Requested ${input.title}`);
        }}
      />
    </Suspense>
  );
}
