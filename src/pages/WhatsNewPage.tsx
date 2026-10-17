import { Ellipsis, Pause, Play } from "lucide-react";
import { useEffect, useState } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { addedLabel } from "../lib/relativeTime";
import type { Album, NewRelease, WhatsNew } from "../lib/types";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { isFollowing, setFollow, startSessionItem } from "../stores/social";
import { useContextState } from "./cards";
import "../shell/social.css";
import { LoadError } from "../components/LoadError";

/** "Album", "EP", "Single". */
export function releaseType(a: Pick<Album, "format">): string {
  return a.format === "ep" ? "EP" : a.format === "single" ? "Single" : "Album";
}

function ReleaseRow({ item }: { item: NewRelease }) {
  const a = item.album;
  const { isContext, playing } = useContextState("album", a.id);
  const play = () => (isContext ? player.toggle() : player.playAlbum(a.id));
  const menu = (el: HTMLElement) =>
    openContextMenu(
      `Options for ${a.title}`,
      [
        { label: "Play", onSelect: () => void player.playAlbum(a.id) },
        { label: "Add to queue", onSelect: () => void player.queueAlbum(a.id) },
        { label: `Go to ${a.artistTitle}`, onSelect: () => nav.go({ name: "artist", id: a.artistId }) },
        startSessionItem(() => void player.playAlbum(a.id)),
        {
          label: "Unfollow",
          when: () => isFollowing(a.artistId),
          separator: true,
          onSelect: () => void setFollow({ id: a.artistId, title: a.artistTitle }, false),
        },
      ],
      el,
    );
  return (
    <li className="release-row">
      <button type="button" className="release-row__art" aria-label={`Open ${a.title}`} onClick={() => nav.go({ name: "album", id: a.id })}>
        <Art path={a.thumb} size={88} />
      </button>
      <div className="release-row__text">
        <button type="button" className="release-row__title truncate" onClick={() => nav.go({ name: "album", id: a.id })}>
          {a.title}
        </button>
        <button type="button" className="release-row__artist truncate" onClick={() => nav.go({ name: "artist", id: a.artistId })}>
          {a.artistTitle}
        </button>
        <span className="release-row__meta">
          {releaseType(a)} • {addedLabel(a.addedAt)}
        </span>
      </div>
      <div className="release-row__actions">
        <IconButton label={`More options for ${a.title}`} icon={<Ellipsis size={20} />} onClick={(e) => menu(e.currentTarget)} />
        <IconButton
          label={playing ? `Pause ${a.title}` : `Play ${a.title}`}
          variant="accent"
          size="lg"
          icon={playing ? <Pause size={20} fill="currentColor" /> : <Play size={20} fill="currentColor" className="nudge-right" />}
          onClick={play}
        />
      </div>
    </li>
  );
}

function Group({ title, items }: { title: string; items: NewRelease[] }) {
  if (!items.length) return null;
  const id = `whats-new-${title.toLowerCase()}`;
  return (
    <section className="release-group" aria-labelledby={id}>
      <h2 id={id} className="section-title">
        {title}
      </h2>
      <ul className="release-list">
        {items.map((i) => (
          <ReleaseRow key={i.album.id} item={i} />
        ))}
      </ul>
    </section>
  );
}

/** Releases added to the library by artists you follow: New, then Earlier. */
export function WhatsNewPage() {
  const [data, setData] = useState<WhatsNew | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let alive = true;
    setError(null);
    backend.whatsNew().then(
      (w) => {
        if (!alive) return;
        setData(w);
        // Shown: what's here stops being new (it stays in New until the page is left).
        if (w.unseen > 0) void backend.whatsNewSeen().catch(() => {});
      },
      (e) => alive && setError(String(e?.message ?? e)),
    );
    return () => {
      alive = false;
    };
  }, [attempt]);

  return (
    <div className="page-pad whats-new">
      <h1 className="page-title">What's New</h1>
      <p className="whats-new__lede">
        {data && !data.following
          ? "New in your library. Follow artists to see only their releases here."
          : "The latest releases from artists you follow."}
      </p>
      {error && <LoadError what="What's New" error={error} onRetry={() => setAttempt((n) => n + 1)} inline />}
      {!data && !error && (
        <ul className="release-list" aria-hidden="true">
          {[0, 1, 2].map((i) => (
            <li key={i} className="release-row">
              <Skeleton width={88} height={88} />
              <div className="release-row__text">
                <Skeleton width="40%" height={16} />
                <Skeleton width="25%" height={14} style={{ marginTop: 8 }} />
              </div>
            </li>
          ))}
        </ul>
      )}
      {data && data.items.length === 0 && (
        <p className="whats-new__empty" role="status">
          Nothing new yet. When an artist you follow gets new music in your library, it shows up here.
        </p>
      )}
      {data && (
        <>
          <Group title="New" items={data.items.filter((i) => i.new)} />
          <Group title="Earlier" items={data.items.filter((i) => !i.new)} />
        </>
      )}
    </div>
  );
}
