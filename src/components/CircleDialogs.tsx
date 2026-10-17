/**
 * Shared playlist and Blend dialogs, mounted once in the shell: Invite
 * collaborators, Playlist permissions, Leave playlist and Leave Blend, and
 * the invites that came straight to you. Which one is open lives in
 * stores/circle.
 */
import { Check, ChevronDown, Link2, Lock, Globe, UserPlus, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { backend } from "../lib/backend";
import { plural } from "../lib/format";
import type { CircleAccount, CirclePerson, SharedSummary } from "../lib/types";
import {
  acceptIncoming,
  copyInviteLink,
  invitePerson,
  leaveBlend,
  leavePlaylist,
  openCircleDialog,
  removePerson,
  setPublic,
  setRole,
  useCircle,
  useShared,
} from "../stores/circle";
import { Button, IconButton } from "./Button";
import { openContextMenu } from "./ContextMenu";
import { Dialog } from "./Dialog";
import { Face } from "./Faces";
import "./circle.css";

export function CircleDialogs() {
  const d = useCircle((s) => s.dialog);
  return (
    <>
      {d?.kind === "invite" && <InviteDialog playlistId={d.playlistId} title={d.title} />}
      {d?.kind === "permissions" && <PermissionsDialog playlistId={d.playlistId} />}
      {d?.kind === "leave" && <LeaveDialog playlistId={d.playlistId} />}
      {d?.kind === "leaveBlend" && <LeaveBlendDialog id={d.id} title={d.title} />}
      <IncomingInvites />
    </>
  );
}

const close = () => openCircleDialog(null);

const ROLE_LABEL: Record<string, string> = {
  owner: "Owner",
  collaborator: "Collaborator",
  listener: "Listener",
  follower: "Follower",
};

// ---------------------------------------------------------------- invite

type Pickable = "collaborator" | "listener";

function RoleChoice({ value, onChange, name }: { value: Pickable; onChange: (r: Pickable) => void; name: string }) {
  return (
    <div className="role-choice" role="radiogroup" aria-label={`Role for ${name}`}>
      {(
        [
          ["collaborator", "Collaborator", "Can add songs and remove the ones they added"],
          ["listener", "Listener", "Can play it and sees it in their library"],
        ] as const
      ).map(([r, label, hint]) => (
        <button key={r} type="button" role="radio" aria-checked={value === r} className={`role-choice__opt ${value === r ? "is-on" : ""}`} onClick={() => onChange(r)}>
          <span className="role-choice__label">{label}</span>
          <span className="role-choice__hint">{hint}</span>
        </button>
      ))}
    </div>
  );
}

function InviteDialog({ playlistId, title }: { playlistId: string; title: string }) {
  const [people, setPeople] = useState<CircleAccount[] | null>(null);
  const [picked, setPicked] = useState<CircleAccount | null>(null);
  const [role, setRoleChoice] = useState<Pickable>("listener");
  const [linkRole, setLinkRole] = useState<Pickable>("listener");
  const [invited, setInvited] = useState<Set<string>>(new Set());
  const shared = useShared(playlistId);
  useEffect(() => {
    let alive = true;
    backend.circlePeople().then(
      (p) => alive && setPeople(p),
      () => alive && setPeople([]),
    );
    return () => {
      alive = false;
    };
  }, []);
  const already = new Set((shared?.people ?? []).map((p) => p.name));
  const home = (people ?? []).filter((p) => p.home);
  const server = (people ?? []).filter((p) => !p.home);
  const send = async () => {
    if (!picked) return;
    if (await invitePerson(playlistId, picked.key, picked.name, role)) {
      setInvited((s) => new Set(s).add(picked.key));
      setPicked(null);
    }
  };
  const group = (label: string, list: CircleAccount[]) =>
    list.length > 0 && (
      <section className="invite__group" aria-label={label}>
        <h3 className="invite__heading">{label}</h3>
        <ul className="invite__people">
          {list.map((p) => {
            const done = invited.has(p.key) || already.has(p.name);
            const open = picked?.key === p.key;
            return (
              <li key={p.key} className={`invite__person ${open ? "is-open" : ""}`}>
                <button type="button" className="invite__pick" aria-expanded={open} disabled={done} onClick={() => setPicked(open ? null : p)}>
                  <Face person={p} size={32} />
                  <span className="invite__name truncate">{p.name}</span>
                  {done ? (
                    <span className="invite__done">
                      <Check size={14} aria-hidden="true" /> {already.has(p.name) ? "On it" : "Invited"}
                    </span>
                  ) : (
                    <ChevronDown size={16} aria-hidden="true" className="invite__chev" />
                  )}
                </button>
                {open && (
                  <div className="invite__role">
                    <RoleChoice value={role} onChange={setRoleChoice} name={p.name} />
                    <Button variant="primary" size="sm" onClick={() => void send()}>
                      Invite {p.name}
                    </Button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      </section>
    );
  return (
    <Dialog open onClose={close} label={`Invite collaborators to ${title}`} className="invite">
      <header className="invite__top">
        <h2 className="confirm__title">Invite to {title}</h2>
        <IconButton label="Close" size="sm" icon={<X size={18} />} onClick={close} />
      </header>
      <p className="confirm__body">They play it from the servers they already reach. Only song details are shared, never your music or your server.</p>
      {people === null ? (
        <p className="invite__note" role="status">
          Looking for people on your server…
        </p>
      ) : people.length === 0 ? (
        <p className="invite__note">Nobody else is on your server or Plex Home. Send an invite link instead.</p>
      ) : (
        <>
          {group("Your Plex Home", home)}
          {group("On your server", server)}
        </>
      )}
      <section className="invite__link" aria-label="Invite link">
        <div className="invite__link-text">
          <span className="invite__link-title">Invite with a link</span>
          <span className="invite__link-hint">Works once, for one person, and runs out after a while.</span>
        </div>
        <label className="invite__toggle">
          <input type="checkbox" checked={linkRole === "collaborator"} onChange={(e) => setLinkRole(e.target.checked ? "collaborator" : "listener")} />
          <span>They can add songs</span>
        </label>
        <Button variant="secondary" icon={<Link2 size={16} />} onClick={() => void copyInviteLink(playlistId, linkRole)}>
          Copy invite link
        </Button>
      </section>
    </Dialog>
  );
}

// ---------------------------------------------------------------- permissions

function PermissionsDialog({ playlistId }: { playlistId: string }) {
  const s = useShared(playlistId);
  if (!s) return null;
  const owner = s.role === "owner";
  const row = (p: CirclePerson) => {
    const name = p.you ? `${p.name} (you)` : p.name;
    let right;
    if (p.role === "owner") right = <span className="perm__role">Owner</span>;
    else if (p.you)
      right = (
        <button
          type="button"
          className="perm__role perm__role--button"
          aria-haspopup="menu"
          aria-label={`Your role: ${ROLE_LABEL[p.role ?? "listener"]}`}
          onClick={(e) => openContextMenu("Your role", [{ label: "Leave playlist", danger: true, onSelect: () => openCircleDialog({ kind: "leave", playlistId }) }], e.currentTarget)}
        >
          {ROLE_LABEL[p.role ?? "listener"]}
          <ChevronDown size={14} aria-hidden="true" />
        </button>
      );
    else if (owner)
      right = (
        <button
          type="button"
          className="perm__role perm__role--button"
          aria-haspopup="menu"
          aria-label={`${p.name}: ${ROLE_LABEL[p.role ?? "listener"]}`}
          onClick={(e) =>
            openContextMenu(
              `${p.name}'s role`,
              [
                { label: "Make collaborator", checked: p.role === "collaborator", onSelect: () => void setRole(playlistId, p, "collaborator") },
                { label: "Make listener", checked: p.role === "listener", onSelect: () => void setRole(playlistId, p, "listener") },
                { label: "Remove from playlist", separator: true, danger: true, onSelect: () => void removePerson(playlistId, p) },
              ],
              e.currentTarget,
            )
          }
        >
          {ROLE_LABEL[p.role ?? "listener"]}
          <ChevronDown size={14} aria-hidden="true" />
        </button>
      );
    else right = <span className="perm__role">{ROLE_LABEL[p.role ?? "listener"]}</span>;
    return (
      <li key={p.id} className="perm__person">
        <Face person={p} size={40} />
        <span className="perm__who">
          <span className="perm__name truncate">{name}</span>
          {p.added > 0 && <span className="perm__added">{plural(p.added, "song")} added</span>}
        </span>
        {right}
      </li>
    );
  };
  return (
    <Dialog open onClose={close} label="Playlist permissions" className="perm">
      <header className="invite__top">
        <h2 className="confirm__title">Playlist permissions</h2>
        <IconButton label="Close" size="sm" icon={<X size={18} />} onClick={close} />
      </header>
      <Visibility summary={s} owner={owner} />
      <h3 className="invite__heading">Shared with</h3>
      <ul className="perm__people">{s.people.map(row)}</ul>
      {owner && (
        <div className="confirm__actions">
          <Button variant="secondary" icon={<UserPlus size={16} />} onClick={() => openCircleDialog({ kind: "invite", playlistId, title: s.title })}>
            Invite
          </Button>
        </div>
      )}
    </Dialog>
  );
}

function Visibility({ summary, owner }: { summary: SharedSummary; owner: boolean }) {
  const opts = [
    { on: false, icon: Lock, label: "Private", hint: "Only people on it can see it." },
    { on: true, icon: Globe, label: "Public", hint: "People who leave can still see it." },
  ];
  return (
    <div className="perm__vis" role="radiogroup" aria-label="Who can see it">
      {opts.map((o) => (
        <button
          key={o.label}
          type="button"
          role="radio"
          aria-checked={summary.public === o.on}
          disabled={!owner}
          className={`perm__vis-opt ${summary.public === o.on ? "is-on" : ""}`}
          onClick={() => summary.public !== o.on && void setPublic(summary.id, o.on)}
        >
          <o.icon size={18} aria-hidden="true" />
          <span className="role-choice__label">{o.label}</span>
          <span className="role-choice__hint">{o.hint}</span>
        </button>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------- leaving

/** What leaving means here, in a sentence. */
export function leaveMessage(s: Pick<SharedSummary, "public" | "role" | "title">): string {
  if (!s.public) return `You'll lose access to ${s.title}. To get it back, you'd need a new invite.`;
  if (s.role === "collaborator") return `You can't add any more songs to ${s.title}. You'll still see it.`;
  return `You'll still see ${s.title}, but you'd lose access if it's made private.`;
}

function LeaveDialog({ playlistId }: { playlistId: string }) {
  const s = useShared(playlistId);
  const notNow = useRef<HTMLButtonElement>(null);
  if (!s) return null;
  return (
    <Dialog open onClose={close} label="Are you sure?" className="confirm" initialFocus={notNow}>
      <h2 className="confirm__title">Are you sure?</h2>
      <p className="confirm__body">{leaveMessage(s)}</p>
      <div className="confirm__actions">
        <Button ref={notNow} variant="ghost" onClick={close}>
          Not now
        </Button>
        <Button
          variant="primary"
          className="btn--danger"
          onClick={() => {
            close();
            void leavePlaylist(playlistId);
          }}
        >
          Leave playlist
        </Button>
      </div>
    </Dialog>
  );
}

function LeaveBlendDialog({ id, title }: { id: string; title: string }) {
  const notNow = useRef<HTMLButtonElement>(null);
  return (
    <Dialog open onClose={close} label="Leave Blend?" className="confirm" initialFocus={notNow}>
      <h2 className="confirm__title">Leave Blend?</h2>
      <p className="confirm__body">{title} is deleted for you and your taste stops being shared. Everyone else's Blend leaves you out from its next update.</p>
      <div className="confirm__actions">
        <Button ref={notNow} variant="ghost" onClick={close}>
          Not now
        </Button>
        <Button
          variant="primary"
          className="btn--danger"
          onClick={() => {
            close();
            void leaveBlend(id, title);
          }}
        >
          Leave Blend
        </Button>
      </div>
    </Dialog>
  );
}

// ---------------------------------------------------------------- invites for you

function IncomingInvites() {
  const incoming = useCircle((s) => s.view.incoming);
  if (!incoming.length) return null;
  return (
    <section className="incoming" aria-label="Invites for you">
      {incoming.slice(0, 3).map((i) => (
        <div key={i.id} className="incoming__card" role="group" aria-label={`Invite from ${i.from}`}>
          <Face person={{ name: i.from, avatar: null }} size={32} />
          <p className="incoming__text">
            <strong>{i.from}</strong> {i.kind === "playlist" ? `invited you to ${i.title}` : "invited you to a Blend"}
            <span className="incoming__note">Not confirmed yet. Platter checks who sent it when you join.</span>
          </p>
          <div className="incoming__actions">
            <Button variant="ghost" size="sm" onClick={() => void backend.circleDismiss(i.id)}>
              Not now
            </Button>
            <Button variant="primary" size="sm" onClick={() => void acceptIncoming(i.id)}>
              Join
            </Button>
          </div>
        </div>
      ))}
    </section>
  );
}
