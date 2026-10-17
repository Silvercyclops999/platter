/**
 * Your profile: your Plex photo and name with a line of what you have and
 * played this month, the Settings cog and a ⋯ menu; this month's minutes,
 * plays and longest streak in one quiet row; your top artists and songs this
 * month (only you see these); your playlists and friends (for the people you
 * share with); and links to all your stats and Wrapped. Nothing here is
 * published: the page is drawn on this computer from what the core already
 * has.
 */
import { Ellipsis, Settings as Cog, UsersRound } from "lucide-react";
import { Fragment, type ReactNode } from "react";
import { Button, IconButton } from "../components/Button";
import { Card, CardSkeleton } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { LikedCover } from "../components/LikedCover";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { metaLine, profileFriends, profilePlaylists, profileSections, thisMonth, type ProfileFriend, type ProfileSection } from "../lib/profile";
import { formatPlays, minutes } from "../lib/stats";
import type { CompareSummary, StatsOverview } from "../lib/statsTypes";
import type { ActivityPanel } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { AvatarFace } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { wrapped } from "../stores/wrapped";
import "./profile.css";
import { LoadError } from "../components/LoadError";

const NONE_YET = new Promise<never>(() => {});

function Section({ id, title, note, action, children }: { id: string; title: string; note?: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="profile-section" aria-labelledby={`profile-${id}`} data-testid={`profile-${id}`}>
      <div className="profile-section__head">
        <div>
          <h2 id={`profile-${id}`} className="section-title profile-section__title">
            {title}
          </h2>
          {note && <p className="profile-section__note">{note}</p>}
        </div>
        {action}
      </div>
      {children}
    </section>
  );
}

function Numbers({ o }: { o: StatsOverview | undefined }) {
  const items: [string, string | null][] = [
    ["Minutes this month", o ? minutes(o.totals.ms).toLocaleString() : null],
    ["Plays", o ? o.totals.plays.toLocaleString() : null],
    ["Longest streak", o ? (o.streak ? `${o.streak.days.toLocaleString()} ${o.streak.days === 1 ? "day" : "days"}` : "None yet") : null],
  ];
  return (
    <dl className="profile-numbers" aria-label="This month" data-testid="profile-numbers">
      {items.map(([label, value]) => (
        <div key={label} className="profile-number">
          <dt className="profile-number__label">{label}</dt>
          <dd className="profile-number__value">{value ?? <Skeleton width={64} height={22} />}</dd>
        </div>
      ))}
    </dl>
  );
}

function TopTracks({ o }: { o: StatsOverview }) {
  const songs = o.topSongs.slice(0, 5);
  const playable = songs.map((s) => s.track?.id).filter((x): x is string => !!x);
  return (
    <TrackList label="Top songs this month" header={false}>
      {songs.map((s, i) =>
        s.track ? (
          <TrackRow key={s.key} track={s.track} index={i + 1} showArt showAlbum={false} compact onPlay={() => void player.playTracks(playable, s.track!.id)} trailing={<span className="profile-plays">{formatPlays(s.plays)}</span>} />
        ) : (
          <div key={s.key} className="stats-row stats-row--gone profile-gone" role="row">
            <span className="profile-gone__num" role="gridcell">
              {i + 1}
            </span>
            <span className="profile-gone__text" role="gridcell">
              <span className="truncate">{s.title}</span>
              <span className="profile-gone__sub truncate">
                {s.artist} · not in your library · {formatPlays(s.plays)}
              </span>
            </span>
          </div>
        ),
      )}
    </TrackList>
  );
}

function FriendCard({ f, compare }: { f: ProfileFriend; compare: boolean }) {
  const initial = f.name.trim().charAt(0).toUpperCase();
  return (
    <li className="profile-friend">
      <AvatarFace path={f.thumb} size={120} className="profile-friend__face" fallback={initial} />
      <span className="profile-friend__name truncate">{f.name}</span>
      {compare && f.canCompare ? (
        <Button size="sm" variant="ghost" aria-label={`Compare with ${f.name}`} onClick={() => nav.go({ name: "stats", tab: "compare", friend: f.name })}>
          Compare
        </Button>
      ) : (
        <span className="profile-friend__sub">Friend</span>
      )}
    </li>
  );
}

function NoHistory() {
  return (
    <div className="profile-empty" role="status" data-testid="profile-empty">
      <span aria-hidden="true">
        <LikedCover icon="stats" size={64} />
      </span>
      <div>
        <h2 className="profile-empty__title">Nothing played yet</h2>
        <p className="profile-empty__text">Play some music, or import your past plays, and this month's listening shows up here.</p>
      </div>
      <Button onClick={() => nav.go({ name: "settings" })}>Import past plays</Button>
    </div>
  );
}

export function ProfilePage() {
  const username = useApp((s) => s.auth?.username ?? null);
  const showAvatar = useSettings((s) => s.settings?.showPlexAvatar !== false);
  const avatar = useApp((s) => (showAvatar ? (s.auth?.avatar ?? null) : null));
  const statsOn = useSettings((s) => s.settings?.statsPage !== false);
  const wrappedOn = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const compareOn = useSettings((s) => !!s.settings?.statsCompare);
  const activityOn = useSettings((s) => s.settings?.showFriendActivity !== false);

  const span = useLoad(() => (statsOn ? backend.statsSpan() : NONE_YET), [statsOn]);
  const today = span.data?.today ?? null;
  const hasHistory = !!span.data?.first;
  const month = useLoad(() => (statsOn && today && hasHistory ? backend.statsOverview(thisMonth(today), 10) : NONE_YET), [statsOn, today, hasHistory]);
  const playlists = useLoad(() => backend.playlists(), []);
  const activity = useLoad<ActivityPanel>(() => (activityOn ? backend.friendActivity() : Promise.resolve({ state: "off", friends: [] })), [activityOn]);
  const shared = useLoad<CompareSummary[]>(() => (compareOn ? backend.statsFriends() : Promise.resolve([])), [compareOn]);

  const name = username ?? "You";
  const initial = name.trim().charAt(0).toUpperCase();
  const o = month.data;
  const lists = playlists.data ? profilePlaylists(playlists.data) : null;
  // Either list failing (offline, not the server's owner) leaves the other.
  const people = activity.data?.friends ?? (activity.error ? [] : null);
  const sharers = shared.data ?? (shared.error ? [] : null);
  const friends = people && sharers ? profileFriends(people, sharers) : null;
  const monthKey = (today ?? new Date().toISOString().slice(0, 10)).slice(0, 7);
  const year = Number(monthKey.slice(0, 4));
  const meta = metaLine(lists?.length ?? null, statsOn ? (o?.totals.plays ?? (span.data && !hasHistory ? 0 : null)) : null);

  const more = (e: React.MouseEvent<HTMLButtonElement>) =>
    openContextMenu(
      "Profile options",
      [
        ...(statsOn ? [{ label: "See all your stats", onSelect: () => nav.go({ name: "stats" }) }] : []),
        ...(wrappedOn ? [{ label: "Your Top Songs", onSelect: () => nav.go({ name: "topSongs", year }) }] : []),
        { label: showAvatar ? "Hide my Plex picture" : "Show my Plex picture", onSelect: () => void updateSettings({ showPlexAvatar: !showAvatar }) },
      ],
      e.currentTarget,
    );

  const sections: Record<ProfileSection, ReactNode> = {
    numbers: statsOn && hasHistory ? <Numbers o={o} /> : null,
    topArtists:
      statsOn && hasHistory ? (
        <Section id="artists" title="Top artists this month" note="Only visible to you" action={<ShowAll label="Show all" onClick={() => nav.go({ name: "stats", tab: "overview", month: monthKey })} />}>
          {!o ? (
            <ul className="profile-cards" aria-hidden="true">
              {[0, 1, 2, 3, 4].map((i) => (
                <li key={i}>
                  <CardSkeleton kind="artist" />
                </li>
              ))}
            </ul>
          ) : o.topArtists.length === 0 ? (
            <p className="profile-muted">No plays yet this month. Play some music and your top artists show up here.</p>
          ) : (
            <ul className="profile-cards" aria-label="Top artists this month">
              {o.topArtists.slice(0, 6).map((a) => (
                <li key={a.key}>
                  <Card kind="artist" title={a.name} subtitle={formatPlays(a.plays)} thumb={a.thumb} onOpen={() => a.artistId && nav.go({ name: "artist", id: a.artistId })} />
                </li>
              ))}
            </ul>
          )}
        </Section>
      ) : null,
    topTracks:
      statsOn && hasHistory ? (
        <Section
          id="tracks"
          title="Top songs this month"
          note="Only visible to you"
          action={wrappedOn ? <ShowAll label="Your Top Songs" onClick={() => nav.go({ name: "topSongs", year })} /> : undefined}
        >
          {!o ? <Skeleton width="100%" height={240} radius="card" /> : o.topSongs.length === 0 ? <p className="profile-muted">No plays yet this month. Play some music and your top songs show up here.</p> : <TopTracks o={o} />}
        </Section>
      ) : null,
    playlists: (
      <Section id="playlists" title="Playlists" action={lists && lists.length > 6 ? <ShowAll label="Show all" onClick={() => nav.go({ name: "library", tab: "playlists" })} /> : undefined}>
        {!lists ? (
          playlists.error ? (
            <LoadError what="your playlists" error={playlists.error} onRetry={playlists.retry} inline />
          ) : (
            <Skeleton width="100%" height={200} radius="card" />
          )
        ) : lists.length === 0 ? (
          <p className="profile-muted">No playlists yet. Make one from the app menu.</p>
        ) : (
          <ul className="profile-cards" aria-label="Playlists">
            {lists.slice(0, 6).map((p) => (
              <li key={p.id}>
                <Card kind="playlist" title={p.title} subtitle={p.trackCount === 1 ? "1 song" : `${p.trackCount.toLocaleString()} songs`} thumb={p.thumb} onOpen={() => nav.go({ name: "playlist", id: p.id })} />
              </li>
            ))}
          </ul>
        )}
      </Section>
    ),
    friends:
      friends && friends.length > 0 ? (
        <Section id="friends" title="Friends">
          <ul className="profile-friends" aria-label="Friends">
            {friends.map((f) => (
              <FriendCard key={f.name} f={f} compare={compareOn && statsOn} />
            ))}
          </ul>
        </Section>
      ) : null,
    links:
      statsOn || wrappedOn ? (
        <nav className="profile-links" aria-label="More of your listening">
          {statsOn && (
            <Button variant="secondary" onClick={() => nav.go({ name: "stats" })}>
              See all your stats
            </Button>
          )}
          {wrappedOn && (
            <Button variant="secondary" onClick={() => wrapped.open(year)}>
              {year} Wrapped
            </Button>
          )}
        </nav>
      ) : null,
  };

  return (
    <div className="page-pad profile" data-testid="profile">
      <header className="profile-head">
        <AvatarFace path={avatar} size={232} className="profile-head__face" fallback={initial || <UsersRound size={56} />} />
        <div className="profile-head__text">
          <p className="profile-head__eyebrow">Profile</p>
          <h1 className="profile-head__name">{name}</h1>
          <p className="profile-head__meta" data-testid="profile-meta">
            {meta || " "}
          </p>
        </div>
      </header>
      <div className="profile-actions">
        <IconButton label="Settings" size="lg" icon={<Cog size={22} />} onClick={() => nav.go({ name: "settings" })} />
        <IconButton label="More options for your profile" size="lg" icon={<Ellipsis size={22} />} onClick={more} />
      </div>
      {statsOn && span.data && !hasHistory && <NoHistory />}
      {profileSections("me").map((s) => (
        <Fragment key={s}>{sections[s]}</Fragment>
      ))}
    </div>
  );
}

function ShowAll({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button type="button" className="profile-section__link" onClick={onClick}>
      {label}
    </button>
  );
}
