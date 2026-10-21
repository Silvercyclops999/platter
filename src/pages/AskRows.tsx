/**
 * Requests on What's New. The owner sees what people asked for, with No
 * thanks and Get it; someone the server is shared with sees their own
 * requests and where each one stands.
 */
import { Check, Clock, Download, Ellipsis, Search } from "lucide-react";
import { Suspense, lazy, useEffect, useState } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { timeAgo } from "../lib/relativeTime";
import type { AskItem, AskState, IncomingAsk, LidarrRequestInput, MyAsk, RequestRuleKind, Settings } from "../lib/types";
import { answerAsk, cancelAsk, followAsks, loadIncoming, useAsks, waiting } from "../stores/asks";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { askLabel } from "./RequestButton";
import "./lidarr-request.css";

const LidarrRequestDialog = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

/** The album name shown for a request (a song's album, else its title). */
export const askAlbum = (i: AskItem) => (i.song && i.album?.trim() ? i.album : i.title);

/** "First Frost" or "Dawn, from Second Wind". */
export const askWhat = (i: AskItem) => (i.song && i.album?.trim() ? `${i.title}, from ${i.album}` : i.title);

/** The art path for a request's cover (a web address the art cache fetches). */
export const askArt = (i: AskItem) => (i.thumb ? (i.thumb.startsWith("/ext/") ? i.thumb : `/ext/${i.thumb}`) : null);

/** What the owner lets a person do ("ask" when nothing's set). */
export function ruleFor(s: Settings | null | undefined, userId: string): RequestRuleKind {
  return s?.requestRules?.find((r) => r.userId === userId)?.rule ?? "ask";
}

/** Set a person's choice; asking first is the default, so it isn't kept. */
export function setRule(userId: string, rule: RequestRuleKind) {
  const all = (useSettings.getState().settings?.requestRules ?? []).filter((r) => r.userId !== userId);
  return updateSettings({ requestRules: rule === "ask" ? all : [...all, { userId, rule }] });
}

export const RULE_LABEL: Record<RequestRuleKind, string> = { off: "Can't ask", ask: "Asks first", straight: "Straight away" };

export function StatePill({ state, progress, note }: { state: AskState; progress: number | null; note: string | null }) {
  const Icon = state === "ready" ? Check : state === "getting" ? Download : state === "notFound" ? Search : Clock;
  const cls = state === "ready" ? "imported" : state === "refused" || state === "failed" ? "failed" : state === "getting" ? "downloading" : state === "notFound" ? "notFound" : "requested";
  return (
    <span className={`lidarr-state lidarr-state--${cls} get-state`} role="status" title={note ?? undefined}>
      {state !== "refused" && state !== "failed" && state !== "declined" && <Icon size={14} aria-hidden="true" />}
      <span className="get-state__text">{state === "declined" ? "Said no" : askLabel(state, progress)}</span>
    </span>
  );
}

/** No thanks and Get it for one request, with the first-request confirm when Get needs it. */
export function useAnswer(a: IncomingAsk) {
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState<LidarrRequestInput | null>(null);
  const album = askAlbum(a.item);
  const yes = async () => {
    setBusy(true);
    const out = await answerAsk(a, true);
    setBusy(false);
    // Your own first request ever is confirmed by typing the album's name, once.
    if (out?.needs?.kind === "firstTime" && a.releaseGroupId) setConfirm({ releaseGroupId: a.releaseGroupId, title: album, artist: a.item.artist, artistId: null, artistMbid: a.item.artistMbid ?? null });
  };
  const no = async () => {
    setBusy(true);
    await answerAsk(a, false);
    setBusy(false);
  };
  const confirmEl = confirm ? (
    <Suspense fallback={null}>
      <LidarrRequestDialog
        open
        input={confirm}
        onClose={() => setConfirm(null)}
        onDone={() => {
          setConfirm(null);
          // Sent: the request follows it from here.
          void answerAsk(a, true);
        }}
      />
    </Suspense>
  ) : null;
  return { busy, yes, no, confirmEl };
}

/** One request someone made of you. */
export function IncomingRow({ a }: { a: IncomingAsk }) {
  const settings = useSettings((s) => s.settings);
  const { busy, yes, no, confirmEl } = useAnswer(a);
  const album = askAlbum(a.item);
  const open = () => a.releaseGroupId && nav.go({ name: "catalogueAlbum", id: a.releaseGroupId });
  const rule = ruleFor(settings, a.personId);
  const menu = (el: HTMLElement) =>
    openContextMenu(
      `Options for ${a.personName}'s request`,
      [
        ...(a.releaseGroupId ? [{ label: "Go to album", onSelect: open }] : []),
        { label: `Always yes for ${a.personName}`, checked: rule === "straight", separator: true, onSelect: () => void setRule(a.personId, rule === "straight" ? "ask" : "straight") },
        { label: `Never ask from ${a.personName}`, checked: rule === "off", onSelect: () => void setRule(a.personId, rule === "off" ? "ask" : "off") },
      ],
      el,
    );
  const meta = [`${a.personName} requested`, a.item.artist, timeAgo(a.createdAt / 1000)].join(" · ");
  return (
    <li className="release-row ask-row">
      <button type="button" className="release-row__art" aria-label={`Open ${album}`} onClick={open}>
        <Art path={askArt(a.item)} size={88} />
      </button>
      <div className="release-row__text">
        <button type="button" className="release-row__title truncate" onClick={open}>
          {askWhat(a.item)}
        </button>
        <span className="release-row__meta truncate">{meta}</span>
        {a.note && <span className="ask-row__note">{a.note}</span>}
      </div>
      <div className="release-row__actions">
        {waiting(a) ? (
          <>
            <Button variant="ghost" size="sm" disabled={busy} onClick={() => void no()} aria-label={`No thanks to ${a.personName}'s request for ${album}`}>
              No thanks
            </Button>
            <Button variant="primary" size="sm" icon={<Download size={14} />} disabled={busy} onClick={() => void yes()} aria-label={`Get ${album} for ${a.personName}`}>
              Get it
            </Button>
          </>
        ) : (
          <StatePill state={a.state} progress={a.progress} note={a.note} />
        )}
        <IconButton label={`More options for ${a.personName}'s request`} icon={<Ellipsis size={20} />} onClick={(e) => menu(e.currentTarget)} />
      </div>
      {confirmEl}
    </li>
  );
}

/** One of my requests. */
export function MyAskRow({ a }: { a: MyAsk }) {
  const album = askAlbum(a.item);
  const open = () => a.item.releaseGroupId && nav.go({ name: "catalogueAlbum", id: a.item.releaseGroupId });
  const canTakeBack = a.state === "sending" || a.state === "requested" || a.state === "declined" || a.state === "refused" || a.state === "failed" || a.state === "notFound";
  return (
    <li className="release-row ask-row">
      <button type="button" className="release-row__art" aria-label={`Open ${album}`} onClick={open}>
        <Art path={askArt(a.item)} size={88} />
      </button>
      <div className="release-row__text">
        <button type="button" className="release-row__title truncate" onClick={open}>
          {askWhat(a.item)}
        </button>
        <span className="release-row__meta truncate">{[a.item.artist, `Requested ${timeAgo(a.createdAt / 1000).toLowerCase()}`].join(" · ")}</span>
        {a.note && <span className="ask-row__note">{a.note}</span>}
      </div>
      <div className="release-row__actions">
        <StatePill state={a.state} progress={a.progress} note={a.note} />
        {canTakeBack && (
          <Button variant="ghost" size="sm" onClick={() => void cancelAsk(a)} aria-label={`${a.state === "sending" || a.state === "requested" ? "Take back" : "Clear"} your request for ${album}`}>
            {a.state === "sending" || a.state === "requested" ? "Take back" : "Clear"}
          </Button>
        )}
      </div>
    </li>
  );
}

/** Settled requests a while back aren't news any more. */
const RECENT_MS = 14 * 24 * 3600 * 1000;

/** What What's New lists: open requests, and settled ones from the last two weeks. */
export function useRequestLists() {
  const incoming = useAsks((s) => s.incoming);
  const view = useAsks((s) => s.view);
  useEffect(() => {
    followAsks();
    void loadIncoming();
  }, []);
  const now = Date.now();
  const recent = (x: { state: AskState; updatedAt: number }) => !(x.state === "ready" || x.state === "declined" || x.state === "refused") || now - x.updatedAt < RECENT_MS;
  return { theirs: (incoming ?? []).filter(recent), mine: view?.guest ? view.mine.filter(recent) : [] };
}

/** The owner's requests from people, then (for someone it's shared with) their own. */
export function RequestsGroup() {
  const { theirs, mine } = useRequestLists();
  return (
    <>
      {theirs.length > 0 && (
        <section className="release-group" aria-labelledby="whats-new-requested">
          <h2 id="whats-new-requested" className="section-title">
            Requested
          </h2>
          <ul className="release-list">
            {theirs.map((a) => (
              <IncomingRow key={a.id} a={a} />
            ))}
          </ul>
        </section>
      )}
      {mine.length > 0 && (
        <section className="release-group" aria-labelledby="whats-new-my-requests">
          <h2 id="whats-new-my-requests" className="section-title">
            Your requests
          </h2>
          <ul className="release-list">
            {mine.map((a) => (
              <MyAskRow key={a.id} a={a} />
            ))}
          </ul>
        </section>
      )}
    </>
  );
}
