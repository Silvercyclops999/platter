/**
 * You, the phone's fourth tab: your picture and name, this month in three
 * readouts, your year's Wrapped, what you played lately, and the way into
 * Settings. Drawn from what the core already has, like the profile page on
 * a computer; nothing here is published.
 */
import { ChevronRight, Settings as Cog, Sparkles, UsersRound } from "lucide-react";
import { Art } from "../../components/Art";
import { LikedCover } from "../../components/LikedCover";
import { backend } from "../../lib/backend";
import { metaLine, profilePlaylists, thisMonth } from "../../lib/profile";
import { timeAgo } from "../../lib/relativeTime";
import { minutes } from "../../lib/stats";
import type { RecentsEntry } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { AvatarFace } from "../../shell/TopBar";
import { useApp } from "../../stores/app";
import { player } from "../../stores/player";
import { nav, type Route } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { wrapped } from "../../stores/wrapped";
import { tap } from "../bits";

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
  const recents = useLoad(() => backend.recents(20), []);

  const name = username ?? "You";
  const initial = name.trim().charAt(0).toUpperCase();
  const o = month.data;
  const lists = playlists.data ? profilePlaylists(playlists.data) : null;
  const meta = metaLine(lists?.length ?? null, statsOn ? (o?.totals.plays ?? (span.data && !hasHistory ? 0 : null)) : null);
  const year = Number((today ?? new Date().toISOString().slice(0, 10)).slice(0, 4));
  const numbers: [string, string | null][] = [
    ["Minutes", o ? minutes(o.totals.ms).toLocaleString() : null],
    ["Plays", o ? o.totals.plays.toLocaleString() : null],
    ["Day streak", o ? (o.streak ? o.streak.days.toLocaleString() : "0") : null],
  ];
  const topArtist = o?.topArtists[0];
  const diaryOn = useSettings((s) => s.settings?.keepDiary !== false);
  const bookmarksOn = useSettings((s) => s.settings?.bookmarksInLibrary !== false);
  const links: { title: string; route: Route }[] = [
    ...(statsOn ? [{ title: "All your stats", route: { name: "stats" } as Route }] : []),
    ...(wrappedOn ? [{ title: "Your Top Songs", route: { name: "topSongs", year } as Route }] : []),
    { title: "Everything you played", route: { name: "recents" } },
    { title: "Mixes made for you", route: { name: "mixes" } },
    ...(diaryOn ? [{ title: "Listening diary", route: { name: "diary" } as Route }] : []),
    ...(bookmarksOn ? [{ title: "Bookmarks", route: { name: "bookmarks" } as Route }] : []),
  ];
  const tools: { title: string; route: Route }[] = [
    { title: "Library import and Missing", route: { name: "import" } },
    { title: "Library doctor", route: { name: "doctor" } },
    { title: "Create a Blend", route: { name: "blendNew" } },
    { title: "Devices", route: { name: "devices" } },
    { title: "Settings", route: { name: "settings" } },
  ];

  return (
    <div className="ph-page ph-you" data-testid="phone-you">
      <header className="ph-you__head">
        <AvatarFace path={avatar} size={160} className="ph-you__face" fallback={initial || <UsersRound size={32} />} />
        <div className="ph-you__who">
          <h1 className="ph-title ph-you__name">{name}</h1>
          <p className="ph-you__meta tabular">{meta || " "}</p>
        </div>
        <button
          type="button"
          className="ph-icon-btn ph-you__cog"
          aria-label="Settings"
          onClick={() => {
            tap();
            nav.go({ name: "settings" });
          }}
        >
          <Cog size={22} aria-hidden="true" />
        </button>
      </header>

      {statsOn && (
        <section className="ph-you__month" aria-labelledby="ph-you-month">
          <h2 id="ph-you-month" className="ph-label">
            This month
          </h2>
          {span.data && !hasHistory ? (
            <p className="ph-rows__note">Nothing played yet. Play a record and this month's listening shows up here.</p>
          ) : (
            <>
              <dl className="ph-readouts" data-testid="you-numbers">
                {numbers.map(([label, value]) => (
                  <div key={label} className="ph-readout">
                    <dd className="ph-readout__value">{value ?? <span className="ph-readout__wait" />}</dd>
                    <dt className="ph-readout__label">{label}</dt>
                  </div>
                ))}
              </dl>
              {topArtist && (
                <p className="ph-you__top">
                  Most played: <strong>{topArtist.name}</strong>
                </p>
              )}
            </>
          )}
        </section>
      )}

      {wrappedOn && (
        <button type="button" className="ph-wrapped" onClick={() => wrapped.open(year)}>
          <span className="ph-wrapped__year tabular" aria-hidden="true">
            {year}
          </span>
          <span className="ph-wrapped__text">
            <span className="ph-wrapped__title">Your {year} Wrapped</span>
            <span className="ph-wrapped__sub">Your year in records, songs and hours</span>
          </span>
          <Sparkles size={20} aria-hidden="true" className="ph-wrapped__icon" />
        </button>
      )}

      <section aria-labelledby="ph-you-recent">
        <h2 id="ph-you-recent" className="ph-section-title">
          Recently played
        </h2>
        {recents.error && !recents.data ? (
          <p className="ph-rows__note">Couldn't load what you played lately.</p>
        ) : !recents.data ? (
          <ul className="ph-rows" aria-busy="true">
            {[0, 1, 2].map((i) => (
              <li key={i} className="ph-lib-row--ghost" />
            ))}
          </ul>
        ) : recents.data.length === 0 ? (
          <p className="ph-rows__note">Nothing played on this phone yet.</p>
        ) : (
          <ul className="ph-rows" aria-label="Recently played">
            {latestOnce(recents.data).map((r) => {
              return (
                <li key={r.key}>
                  <button type="button" className="ph-row" onClick={r.open}>
                    {r.liked ? <LikedCover size={48} /> : <Art path={r.thumb} size={48} round={r.round} kind={r.round ? "artist" : "album"} monogram={r.round ? r.title : undefined} />}
                    <span className="ph-row__text">
                      <span className="ph-row__title">{r.title}</span>
                      <span className="ph-row__sub">{r.sub}</span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <ul className="ph-set-list ph-you__links" aria-label="More">
        {links.map((l) => (
          <li key={l.title}>
            <button type="button" className="ph-set-link" onClick={() => nav.go(l.route)}>
              <span className="ph-set-link__text">
                <span className="ph-set-link__title">{l.title}</span>
              </span>
              <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
            </button>
          </li>
        ))}
      </ul>

      <ul className="ph-set-list ph-you__links" aria-label="Tools">
        {tools.map((l) => (
          <li key={l.title}>
            <button type="button" className="ph-set-link" onClick={() => nav.go(l.route)}>
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
