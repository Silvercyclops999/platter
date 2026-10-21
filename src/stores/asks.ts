/**
 * Requests ("Ask for music", docs/IPC.md `asks_*`). On a server someone
 * shares with you, Request shows where Get would and the request goes to
 * the owner; the owner sees what people asked for on What's New and says
 * yes (through Get, with every check) or no.
 */
import { useEffect } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { AppError, AskItem, AsksView, IncomingAsk, MyAsk } from "../lib/types";
import { toast } from "./toasts";

interface AsksState {
  view: AsksView | null;
  incoming: IncomingAsk[] | null;
}

export const useAsks = create<AsksState>(() => ({ view: null, incoming: null }));

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

const fold = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
const albumOf = (i: AskItem) => (i.song && i.album?.trim() ? i.album : i.title);

export async function loadAsksView() {
  try {
    useAsks.setState({ view: await backend.asksView() });
  } catch {
    useAsks.setState({ view: { guest: false, canRequest: false, straightAway: false, mine: [] } });
  }
}

export async function loadIncoming() {
  try {
    useAsks.setState({ incoming: await backend.asksIncoming() });
  } catch {
    useAsks.setState({ incoming: [] });
  }
}

let listening = false;

/** Test hook: forget what was loaded. */
export function resetAsks() {
  useAsks.setState({ view: null, incoming: null });
}

/** Follow the core's requests (once). */
export function followAsks() {
  if (!useAsks.getState().view) void loadAsksView();
  if (listening) return;
  listening = true;
  void backend.listen("asks://mine", () => void loadAsksView());
  void backend.listen("asks://incoming", (all) => useAsks.setState({ incoming: all }));
  // A new sign-in or server: who may ask changes.
  void backend.listen("auth://changed", () => {
    void loadAsksView();
    void loadIncoming();
  });
}

/** Request shows where Get would (signed in as someone the server is shared with, and allowed). */
export function useCanRequest(): boolean {
  useEffect(followAsks, []);
  return useAsks((s) => s.view?.canRequest === true);
}

/** Signed in as someone the server is shared with: Lidarr setup notes don't apply. */
export function useIsGuest(): boolean {
  useEffect(followAsks, []);
  return useAsks((s) => s.view?.guest === true);
}

/** My request for an album, by release group or by its name and artist. */
export function useMyAsk(album: { releaseGroupId?: string | null; title: string; artist: string }): MyAsk | null {
  return useAsks((s) => {
    const mine = s.view?.mine ?? [];
    const rg = album.releaseGroupId?.toLowerCase();
    return (
      mine.find((a) => rg && a.item.releaseGroupId?.toLowerCase() === rg) ??
      mine.find((a) => fold(albumOf(a.item)) === fold(album.title) && fold(a.item.artist) === fold(album.artist)) ??
      null
    );
  });
}

function upsertMine(a: MyAsk) {
  const v = useAsks.getState().view ?? { guest: true, canRequest: true, straightAway: false, mine: [] };
  useAsks.setState({ view: { ...v, mine: [a, ...v.mine.filter((x) => x.id !== a.id)] } });
}

/** Request something from the server's owner. */
export async function requestItem(item: AskItem): Promise<MyAsk | null> {
  try {
    const a = await backend.asksRequest(item);
    upsertMine(a);
    return a;
  } catch (e) {
    toast.error(errText(e) || `Couldn't request ${albumOf(item)}. Try again in a moment`);
    return null;
  }
}

/** Take a request back. */
export async function cancelAsk(a: MyAsk) {
  try {
    await backend.asksCancel(a.id);
    const v = useAsks.getState().view;
    if (v) useAsks.setState({ view: { ...v, mine: v.mine.filter((x) => x.id !== a.id) } });
  } catch (e) {
    toast.error(errText(e) || "Couldn't take it back. Try again in a moment");
  }
}

/** Waiting for the owner's yes. */
export const waiting = (a: IncomingAsk) => a.state === "requested";

/** The owner's yes or no. Resolves with what Get needs first, if anything. */
export async function answerAsk(a: IncomingAsk, yes: boolean) {
  try {
    const out = await backend.asksAnswer(a.id, yes);
    const all = useAsks.getState().incoming ?? [];
    useAsks.setState({ incoming: all.map((x) => (x.id === a.id ? out.incoming : x)) });
    return out;
  } catch (e) {
    toast.error(errText(e) || "Couldn't answer that just now. Try again in a moment");
    return null;
  }
}
