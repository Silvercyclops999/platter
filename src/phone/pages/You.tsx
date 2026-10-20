/**
 * You, the phone's fourth tab: your picture and name, this month in words
 * (the profile page's own sentence), your year's Wrapped as a ruled row
 * with its record, then the same short list as the computer's profile
 * menu: Stats, Devices, Settings and Help. Drawn from what the core already
 * has, like the profile page on a computer; nothing here is published.
 * Keeping devices in sync lives on Devices (the row below is shared with it).
 */
import { ChevronRight, UsersRound } from "lucide-react";
import { backend } from "../../lib/backend";
import { metaLine, profilePlaylists, thisMonth } from "../../lib/profile";
import { timeAgo } from "../../lib/relativeTime";
import type { RecentsEntry } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { MonthStory } from "../../pages/MonthStory";
import { YearEntry } from "../../pages/StatsWrapped";
import { AvatarFace } from "../../shell/TopBar";
import { useApp } from "../../stores/app";
import { player } from "../../stores/player";
import { nav, type Route } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { wrapped } from "../../stores/wrapped";
import { tap } from "../bits";
import { toast } from "../../stores/toasts";
import { failText } from "../../lib/errorText";
import { useState } from "react";
import { useDeviceSync } from "../../pages/KeepInSyncSettings";
import type { DeviceSyncDevice } from "../../lib/types";
import { openSyncStep } from "../SyncSetup";
import "../sync-setup.css";

const NONE_YET = new Promise<never>(() => {});

/** A recent play's row: what it was, and how long ago. */
export function recentInfo(e: RecentsEntry): { key: string; title: string; sub: string; thumb: string | null; round: boolean; liked: boolean; open: () => void } {
  const ago = timeAgo(e.at / 1000);
  const i = e.item;
  if (e.kind === "liked" || e.kind === "likedAdded")
    return { key: `liked:${e.at}`, title: "Liked Songs", sub: e.kind === "likedAdded" ? `Liked ${ago.toLowerCase()}` : ago, thumb: null, round: false, liked: true, open: () => nav.go({ name: "liked" }) };
  if (i?.kind === "album") return { key: `album:${i.album.id}:${e.at}`, title: i.album.title, sub: `${i.album.artistTitle}, ${ago.toLowerCase()}`, thumb: i.album.thumb, round: false, liked: false, open: () => nav.go({ name: "album", id: i.album.id }) };
  if (i?.kind === "artist") return { key: `artist:${i.artist.id}:${e.at}`, title: i.artist.title, sub: `Artist, ${ago.toLowerCase()}`, thumb: i.artist.thumb, round: true, liked: false, open: () => nav.go({ name: "artist", id: i.artist.id }) };
  if (i?.kind === "playlist")
    return { key: `playlist:${i.playlist.id}:${e.at}`, title: i.playlist.title, sub: `Playlist, ${ago.toLowerCase()}`, thumb: i.playlist.thumb, round: false, liked: false, open: () => nav.go({ name: "playlist", id: i.playlist.id }) };
  const first = e.tracks[0];
  const ids = [...e.tracks].reverse().map((t) => t.id);
  return {
    key: `songs:${e.at}`,
    title: first ? (e.tracks.length === 1 ? first.title : `${first.title} and ${e.tracks.length - 1} more`) : "Songs",
    sub: first ? `${first.artistTitle}, ${ago.toLowerCase()}` : ago,
    thumb: first?.thumb ?? null,
    round: false,
    liked: false,
    open: () => void player.playTracks(ids),
  };
}

/** Each record, artist or playlist once, at its latest play; at most five. */
export function latestOnce(list: RecentsEntry[], max = 5): ReturnType<typeof recentInfo>[] {
  const seen = new Set<string>();
  const out: ReturnType<typeof recentInfo>[] = [];
  for (const e of list) {
    const r = recentInfo(e);
    const id = r.key.startsWith("songs:") ? r.key : r.key.split(":").slice(0, r.key.startsWith("liked:") ? 1 : 2).join(":");
    if (seen.has(id)) continue;
    seen.add(id);
    out.push(r);
    if (out.length >= max) break;
  }
  return out;
}

/** The You tab's short list: the same doors as the computer's profile menu. */
export function youLinks(statsOn: boolean): { title: string; route: Route }[] {
  return [
    ...(statsOn ? [{ title: "Stats", route: { name: "stats" } as Route }] : []),
    { title: "Devices", route: { name: "devices" } },
    { title: "Settings", route: { name: "settings" } },
    { title: "Help", route: { name: "help" } },
  ];
}

export function PhoneYou() {
  const username = useApp((s) => s.auth?.username ?? null);
  const showAvatar = useSettings((s) => s.settings?.showPlexAvatar !== false);
  const avatar = useApp((s) => (showAvatar ? (s.auth?.avatar ?? null) : null));
  const statsOn = useSettings((s) => s.settings?.statsPage !== false);
  const wrappedOn = useSettings((s) => s.settings?.wrappedTopSongs !== false);

  const span = useLoad(() => (statsOn ? backend.statsSpan() : NONE_YET), [statsOn]);
  const today = span.data?.today ?? null;
  const hasHistory = !!span.data?.first;
  const month = useLoad(() => (statsOn && today && hasHistory ? backend.statsOverview(thisMonth(today), 3) : NONE_YET), [statsOn, today, hasHistory]);
  const playlists = useLoad(() => backend.playlists(), []);

  const name = username ?? "You";
  const initial = name.trim().charAt(0).toUpperCase();
  const o = month.data;
  const lists = playlists.data ? profilePlaylists(playlists.data) : null;
  const meta = metaLine(lists?.length ?? null, statsOn ? (o?.totals.plays ?? (span.data && !hasHistory ? 0 : null)) : null);
  const year = Number((today ?? new Date().toISOString().slice(0, 10)).slice(0, 4));
  const links = youLinks(statsOn);

  return (
    <div className="ph-page ph-you" data-testid="phone-you">
      <header className="ph-you__head">
        <AvatarFace path={avatar} size={160} className="ph-you__face" fallback={initial || <UsersRound size={32} />} />
        <div className="ph-you__who">
          <h1 className="ph-title ph-you__name">{name}</h1>
          <p className="ph-you__meta tabular">{meta || " "}</p>
        </div>
      </header>

      {statsOn && (
        <section className="ph-you__month" aria-labelledby="ph-you-month">
          <h2 id="ph-you-month" className="ph-label">
            This month
          </h2>
          {span.data && !hasHistory ? (
            <p className="ph-rows__note">Nothing played yet. Play a record and this month's listening shows up here.</p>
          ) : (
            <MonthStory o={o} testId="you-month" />
          )}
        </section>
      )}

      {wrappedOn && (
        <YearEntry className="ph-you__wrapped" year={year} title={`Your ${year} Wrapped`} shown="Wrapped" sub="Your year in records, songs and hours" onOpen={() => wrapped.open(year)} />
      )}

      <ul className="ph-set-list ph-you__links" aria-label="You">
        {links.map((l) => (
          <li key={l.title}>
            <button
              type="button"
              className="ph-set-link"
              onClick={() => {
                tap();
                nav.go(l.route);
              }}
            >
              <span className="ph-set-link__text">
                <span className="ph-set-link__title">{l.title}</span>
              </span>
              <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** The device to name on the row: the one in step most recently. */
export function syncRowDevice(devices: DeviceSyncDevice[]): DeviceSyncDevice | null {
  return [...devices].sort((a, b) => (b.lastSyncedAt ?? 0) - (a.lastSyncedAt ?? 0))[0] ?? null;
}

/** The row's two lines for a paired device. */
export function syncRowWords(d: DeviceSyncDevice, others: number): { title: string; sub: string; warn: boolean } {
  const inSync = d.words === "In sync";
  const title = inSync ? `In sync with ${d.name}` : d.words;
  const ago = d.lastSyncedAt ? timeAgo(d.lastSyncedAt / 1000) : null;
  const when = ago ? (inSync ? ago : `Last in step ${ago.toLowerCase()}`) : "Not in step yet";
  const more = others > 0 ? `, and ${others === 1 ? "1 more device" : `${others} more devices`}` : "";
  return { title, sub: `${when}${more}`, warn: !inSync };
}

/**
 * Keep my devices in sync, on Devices: before a computer is paired, the way
 * to copy from it; after, how the devices are doing, with Sync now.
 */
export function SyncRow() {
  const [ov, setOv, { failed, reload }] = useDeviceSync();
  const [busy, setBusy] = useState(false);
  const d = syncRowDevice(ov?.devices ?? []);
  const toSettings = () => {
    tap();
    nav.go({ name: "settings", page: "sync" });
  };
  if (!ov && failed)
    return (
      <div className="ph-you__sync" data-testid="you-sync">
        <span className="ph-you__sync-text">
          <span className="ph-you__sync-title">Couldn't read sync status</span>
          <span className="ph-you__sync-sub">Your paired devices are fine.</span>
        </span>
        <button type="button" className="ph-text-btn" onClick={() => void reload()}>
          Retry
        </button>
      </div>
    );
  if (!ov) return <div className="ph-you__sync" data-testid="you-sync" aria-busy="true" />;
  if (!d)
    return (
      <div className="ph-you__sync" data-testid="you-sync">
        <button
          type="button"
          className="ph-you__sync-text"
          onClick={() => {
            tap();
            openSyncStep(false);
          }}
        >
          <span className="ph-you__sync-title">Copy settings from my computer</span>
          <span className="ph-you__sync-sub">Your look, pins and follows, the way Platter is set up there</span>
        </button>
        <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
      </div>
    );
  const w = syncRowWords(d, ov.devices.length - 1);
  const now = () => {
    tap();
    setBusy(true);
    backend.deviceSyncNow().then(
      (o) => {
        setBusy(false);
        setOv(o);
        const first = syncRowDevice(o.devices);
        if (first) toast.info(first.words === "In sync" ? `In sync with ${first.name}` : first.words);
      },
      (e: unknown) => {
        setBusy(false);
        toast.error(failText("Couldn't sync", e));
      },
    );
  };
  return (
    <div className="ph-you__sync" data-testid="you-sync">
      <button type="button" className="ph-you__sync-text" onClick={toSettings} aria-label={`${w.title}. ${w.sub}. Sync settings`}>
        <span className="ph-you__sync-title">{w.title}</span>
        <span className={`ph-you__sync-sub ${w.warn ? "is-warn" : ""}`}>{w.sub}</span>
      </button>
      <button type="button" className="ph-text-btn" onClick={now} disabled={busy || !ov.on}>
        {busy ? "Syncing" : "Sync now"}
      </button>
    </div>
  );
}