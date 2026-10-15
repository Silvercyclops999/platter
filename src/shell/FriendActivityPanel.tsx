import { Music, Play, X } from "lucide-react";
import { useEffect, useState } from "react";
import { IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { shortAgo } from "../lib/relativeTime";
import type { FriendActivity } from "../lib/types";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { loadActivity, useSocial } from "../stores/social";
import { useUi } from "../stores/ui";
import { AvatarFace } from "./TopBar";
import "./social.css";

/** How often the panel reads activity again while it's open. */
export const ACTIVITY_REFRESH_MS = 60_000;

function FriendRow({ f, now }: { f: FriendActivity; now: number }) {
  const when = shortAgo(f.at, now, f.now);
  const openAlbum = () => f.albumId && nav.go({ name: "album", id: f.albumId });
  return (
    <li className="friend-row" aria-label={`${f.name}: ${f.title} by ${f.artist}, ${f.now ? "listening now" : `${when} ago`}`}>
      <div className="friend-row__avatar">
        <AvatarFace path={f.thumb} size={48} className="friend-row__face" fallback={f.name.trim().charAt(0).toUpperCase()} />
        <button type="button" className="friend-row__play" aria-label={`Play ${f.title}`} onClick={() => void player.playTracks([f.trackId], f.trackId)}>
          <Play size={18} fill="currentColor" aria-hidden="true" />
        </button>
        {f.now && <span className="friend-row__live" aria-hidden="true" />}
      </div>
      <div className="friend-row__text">
        <div className="friend-row__top">
          <span className="friend-row__name truncate">{f.name}</span>
          <span className={`friend-row__when ${f.now ? "is-now" : ""}`}>{when}</span>
        </div>
        <div className="friend-row__line truncate">
          <button type="button" className="friend-row__link" onClick={openAlbum} disabled={!f.albumId}>
            {f.title}
          </button>
          {f.artist && <span> • {f.artist}</span>}
        </div>
        {f.album && (
          <div className="friend-row__line friend-row__ctx truncate">
            <Music size={12} aria-hidden="true" />
            <button type="button" className="friend-row__link" onClick={openAlbum} disabled={!f.albumId}>
              {f.album}
            </button>
          </div>
        )}
      </div>
    </li>
  );
}

/** Right panel: what other people on your Plex server listened to lately. */
export function FriendActivityPanel() {
  const panel = useSocial((s) => s.activity);
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    void loadActivity(true);
    const t = setInterval(() => {
      setNow(Date.now() / 1000);
      void loadActivity();
    }, ACTIVITY_REFRESH_MS);
    return () => clearInterval(t);
  }, []);
  const close = () => useUi.getState().setActivityOpen(false);

  let body;
  if (!panel) {
    body = (
      <ul className="friend-list" aria-hidden="true">
        {[0, 1, 2].map((i) => (
          <li key={i} className="friend-row">
            <Skeleton width={48} height={48} radius="circle" />
            <div className="friend-row__text">
              <Skeleton width="40%" height={14} />
              <Skeleton width="70%" height={12} style={{ marginTop: 8 }} />
            </div>
          </li>
        ))}
      </ul>
    );
  } else if (panel.state === "notAdmin") {
    body = (
      <p className="friend-empty" role="status">
        Friend activity isn't available on this server. Plex only shares who's listening with the server's owner.
      </p>
    );
  } else if (panel.state === "offline") {
    body = (
      <p className="friend-empty" role="status">
        Can't reach your server right now. Friend activity comes back when it does.
      </p>
    );
  } else if (panel.state === "off") {
    body = (
      <p className="friend-empty" role="status">
        Friend activity is turned off in Settings.
      </p>
    );
  } else if (panel.friends.length === 0) {
    body = (
      <p className="friend-empty" role="status">
        No one else on your server has listened in the last week.
      </p>
    );
  } else {
    body = (
      <ul className="friend-list" aria-label="Friends' latest songs">
        {panel.friends.map((f) => (
          <FriendRow key={f.accountId} f={f} now={now} />
        ))}
      </ul>
    );
  }

  return (
    <aside className="queue-panel activity-panel" aria-label="Listening activity">
      <header className="queue-panel__head">
        <h2 className="activity-panel__title">Listening activity</h2>
        <IconButton label="Close listening activity" icon={<X size={18} />} onClick={close} />
      </header>
      <div className="queue-panel__body">{body}</div>
    </aside>
  );
}
