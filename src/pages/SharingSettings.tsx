import { Download, LockKeyhole, Share2, UserMinus } from "lucide-react";
import { useEffect } from "react";
import { Button, IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import type { PersonWithAccess } from "../lib/types";
import { InviteCallout, unreleasedLabel } from "../shell/ShareMusic";
import { PersonFace } from "../shell/ListenStrip";
import { updateSettings, useSettings } from "../stores/settings";
import { approveLinkAsk, denyLinkAsk, lastActive, loadInvites, loadLinkAsks, loadOverview, loadPeople, openShareDialog, removeAccess, useSharing } from "../stores/sharing";
import "../shell/share-music.css";

/** "Share my music": the button that opens the share dialog for anyone. */
export function ShareMyMusic({ ids }: { ids: { title: string; desc: string } }) {
  return (
    <Button variant="primary" size="sm" icon={<Share2 size={14} />} aria-describedby={ids.desc} onClick={() => openShareDialog({ kind: "anyone" })}>
      Share my music
    </Button>
  );
}

function PersonRow({ p }: { p: PersonWithAccess }) {
  return (
    <li className="share-person">
      <PersonFace person={{ name: p.name, avatar: null }} size={32} />
      <span className="share-person__text">
        <span className="share-person__name truncate">{p.name}</span>
        <span className="share-person__meta">
          {p.libraries.join(", ") || "No libraries"} · {p.pending ? "Invited, not accepted yet" : `Last active: ${lastActive(p.lastSeenAt)}`}
        </span>
        <span className="share-person__tags">
          {p.unreleasedHidden && (
            <span className="share-tag share-tag--vault">
              <LockKeyhole size={10} aria-hidden="true" />
              Unreleased hidden
            </span>
          )}
          {p.allowDownloads && (
            <span className="share-tag">
              <Download size={10} aria-hidden="true" />
              Can download
            </span>
          )}
          {p.pending && <span className="share-tag share-tag--pending">Pending</span>}
        </span>
      </span>
      <IconButton size="sm" label={`Remove ${p.name}'s access`} icon={<UserMinus size={16} />} onClick={() => void removeAccess(p)} />
    </li>
  );
}

/** People who opened a share link: nothing is shared until you say yes. */
function LinkAsks() {
  const asks = useSharing((s) => s.asks);
  useEffect(() => void loadLinkAsks(), []);
  if (!asks.length) return null;
  return (
    <ul className="share-people" aria-label="Asking for your music">
      {asks.map((a) => (
        <li key={a.id} className="share-person">
          <PersonFace person={{ name: a.name, avatar: null }} size={32} />
          <span className="share-person__text">
            <span className="share-person__name truncate">{a.name}</span>
            <span className="share-person__meta">Plex username {a.user} · opened your share link and asks for your music</span>
          </span>
          <Button variant="ghost" size="sm" onClick={() => void denyLinkAsk(a)}>
            No
          </Button>
          <Button variant="primary" size="sm" onClick={() => void approveLinkAsk(a)}>
            Share
          </Button>
        </li>
      ))}
    </ul>
  );
}

/** People with access: what each can see, when they were last on, remove. */
export function PeopleWithAccess() {
  const people = useSharing((s) => s.people);
  const overview = useSharing((s) => s.overview);
  useEffect(() => {
    void loadOverview().then((o) => (o?.owner ? loadPeople() : useSharing.setState({ people: [] })));
  }, []);
  if (people === null) return <Skeleton width="100%" height={56} radius="card" />;
  if (overview && !overview.owner) return <p className="share-empty">Only the server's owner can share it. {overview.serverName ?? "This server"} is shared with you.</p>;
  if (!people.length)
    return (
      <>
        <LinkAsks />
        <p className="share-empty">Nobody else has access to your server yet.</p>
      </>
    );
  return (
    <>
      <LinkAsks />
      <ul className="share-people" aria-label="People with access">
        {people.map((p) => (
          <PersonRow key={p.id} p={p} />
        ))}
      </ul>
    </>
  );
}

/** Invites to you, accepted here with one click. */
export function InvitesToYou() {
  const invites = useSharing((s) => s.invites);
  useEffect(() => void loadInvites(), []);
  if (!invites.length) return <p className="share-empty">No invites waiting. When a friend shares their server with you, it shows here.</p>;
  return (
    <div className="share-invites">
      {invites.map((i) => (
        <InviteCallout key={i.id} invite={i} compact />
      ))}
    </div>
  );
}

const NONE: string[] = [];

/** "Libraries I share by default": music libraries only, none chosen at first. */
export function DefaultLibraries() {
  const overview = useSharing((s) => s.overview);
  const chosen = useSettings((s) => s.settings?.shareDefaultLibraries ?? NONE);
  useEffect(() => {
    if (!useSharing.getState().overview) void loadOverview();
  }, []);
  if (!overview) return <Skeleton width={220} height={32} radius="pill" />;
  if (!overview.owner || !overview.libraries.length) return null;
  return (
    <div className="share-defaults" role="group" aria-label="Libraries I share by default">
      {overview.libraries.map((l) => {
        const on = chosen.includes(l.id);
        return (
          <label key={l.id} className={`share-lib ${on ? "is-on" : ""}`}>
            <input
              type="checkbox"
              className="share-lib__box"
              checked={on}
              onChange={(e) => void updateSettings({ shareDefaultLibraries: e.target.checked ? [...chosen, l.id] : chosen.filter((x) => x !== l.id) })}
            />
            <span className="share-lib__disc" aria-hidden="true" />
            <span className="share-lib__text">
              <span className="share-lib__title truncate">{l.title}</span>
              <span className={`share-lib__sub ${l.unreleased !== 0 ? "share-lib__sub--vault" : ""}`}>{unreleasedLabel(l.unreleased)}</span>
            </span>
          </label>
        );
      })}
    </div>
  );
}
