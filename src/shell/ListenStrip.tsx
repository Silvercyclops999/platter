import { Copy, Link, RefreshCw, UserMinus, UsersRound, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button, IconButton } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { QrCode } from "../components/QrCode";
import type { ListenPerson, ListenView } from "../lib/types";
import { closeInvite, copyInvite, endSession, listenAlong, openInvite, removePerson, startSession, useSocial } from "../stores/social";
import "./social.css";

/** "Alex", "Alex and Sam", "Alex, Sam and 2 more". */
export function namesLine(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  if (names.length === 3) return `${names[0]}, ${names[1]} and ${names[2]}`;
  return `${names[0]}, ${names[1]} and ${names.length - 2} more`;
}

/** Someone's face: their picture when one is shared, else their name's first letter. */
export function PersonFace({ person, size = 20 }: { person: Pick<ListenPerson, "name" | "avatar">; size?: number }) {
  const letter = [...(person.name.trim() || "?")][0].toUpperCase();
  return (
    <span className="person-face" style={{ width: size, height: size, fontSize: Math.round(size * 0.5) }} aria-hidden="true">
      {person.avatar ? <img src={person.avatar} alt="" /> : letter}
    </span>
  );
}

/** Who added a song in the session, for the queue ("Added by Alex"). */
export function addedBy(v: ListenView, trackId: string, current = false): ListenPerson | null {
  if (v.role === "none") return null;
  const id = current ? v.nowBy : v.queue.find((q) => q.trackId === trackId)?.by;
  return (id && v.people.find((p) => p.id === id)) || null;
}

function stripText(v: ListenView): string {
  const host = v.hostName ?? "the host";
  if (v.role === "host") {
    const others = v.people.filter((p) => !p.me).map((p) => p.name);
    return others.length ? `Listening session: ${namesLine(others)} listening along` : "Listening session: waiting for people to join";
  }
  if (v.status === "hostGone") return `${host} stopped answering`;
  if (v.status === "ended") return `${host} ended the listening session`;
  if (v.status === "removed") return `${host} removed you from the listening session`;
  if (v.status === "connecting") return v.hostName ? `Joining ${host}'s listening session…` : "Joining the listening session…";
  const others = v.people.filter((p) => !p.me && !p.host).map((p) => p.name);
  return others.length ? `Listening along with ${host}, ${namesLine(others)}` : `Listening along with ${host}`;
}

const RELAY_TEXT: Partial<Record<ListenView["relay"], string>> = {
  unreachable: "Can't reach the relay. Trying again…",
  limited: "The relay is busy. Trying again in a while…",
};

/** The strip above the player bar while in a listening session: who's in it, inviting, and how to leave. */
export function ListenStrip() {
  const v = useSocial((s) => s.listen);
  const [peopleOpen, setPeopleOpen] = useState(false);
  const people = useRef<HTMLDivElement>(null);
  // A new session (or none) starts with the list closed.
  useEffect(() => setPeopleOpen(false), [v.sessionId, v.status]);
  useEffect(() => {
    if (!peopleOpen) return;
    const away = (e: MouseEvent) => {
      if (!people.current?.contains(e.target as Node)) setPeopleOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [peopleOpen]);
  if (v.role === "none") return <InviteDialog />;
  const stopped = v.status === "hostGone" || v.status === "ended" || v.status === "removed";
  const relay = !stopped ? RELAY_TEXT[v.relay] : undefined;
  return (
    <div className={`listen-strip ${stopped ? "listen-strip--stopped" : ""}`} role="region" aria-label="Listening session">
      <UsersRound size={14} aria-hidden="true" />
      <span className="listen-strip__faces" aria-hidden="true">
        {v.people.slice(0, 5).map((p) => (
          <PersonFace key={p.id} person={p} size={20} />
        ))}
      </span>
      <span className="listen-strip__text truncate" role="status">
        {stripText(v)}
        {relay && <span className="listen-strip__relay"> · {relay}</span>}
      </span>
      {!stopped && v.people.length > 0 && (
        <div className="listen-people" ref={people}>
          <button type="button" className="listen-strip__btn" aria-expanded={peopleOpen} onClick={() => setPeopleOpen((o) => !o)}>
            People ({v.people.length})
          </button>
          {peopleOpen && <PeopleList v={v} />}
        </div>
      )}
      {v.role === "host" && (
        <button type="button" className="listen-strip__btn" onClick={() => void openInvite()}>
          <Link size={14} aria-hidden="true" />
          Invite
        </button>
      )}
      <button type="button" className="listen-strip__btn" onClick={() => void endSession()}>
        {v.role === "host" ? "End session" : stopped ? "Close" : "Leave"}
      </button>
      <InviteDialog />
    </div>
  );
}

/** Everyone in the session; the host can remove guests. */
function PeopleList({ v }: { v: ListenView }) {
  return (
    <ul className="listen-people__list" aria-label="People in the session">
      {v.people.map((p) => (
        <li key={p.id} className="listen-people__row">
          <PersonFace person={p} size={28} />
          <span className="listen-people__name truncate">
            {p.name}
            {p.me && <span className="listen-people__tag"> (you)</span>}
            {p.host && <span className="listen-people__tag"> · host</span>}
          </span>
          {v.role === "host" && !p.host && (
            <IconButton size="sm" label={`Remove ${p.name}`} icon={<UserMinus size={16} />} onClick={() => void removePerson(p.id)} />
          )}
        </li>
      ))}
    </ul>
  );
}

function expiresIn(at: number): string {
  const h = Math.max(0, Math.round((at - Date.now()) / 3_600_000));
  if (h >= 48) return `${Math.round(h / 24)} days`;
  if (h >= 1) return h === 1 ? "1 hour" : `${h} hours`;
  return "less than an hour";
}

/** The invite: a link and a QR code that work once and run out. */
export function InviteDialog() {
  const open = useSocial((s) => s.inviteOpen);
  const inv = useSocial((s) => s.listen.invite);
  const [copied, setCopied] = useState(false);
  const copyBtn = useRef<HTMLButtonElement>(null);
  useEffect(() => setCopied(false), [inv?.link]);
  return (
    <Dialog open={open} onClose={closeInvite} label="Invite people to listen along" className="confirm invite-dialog" initialFocus={copyBtn}>
      <h2 className="confirm__title">Invite people to listen along</h2>
      {inv ? (
        <>
          <p className="confirm__body">
            Send this link, or let them scan the code. It works for one person, once, and runs out in {expiresIn(inv.expiresAt)}. They need access to
            your Plex library, and they only ever see your name.
          </p>
          <div className="invite-dialog__code">
            {inv.qr && <QrCode qr={inv.qr} label="QR code for the invite link" className="invite-dialog__qr" />}
            <div className="invite-dialog__link">
              <input className="setting-text" readOnly value={inv.link} aria-label="Invite link" onFocus={(e) => e.currentTarget.select()} />
              <Button
                ref={copyBtn}
                size="sm"
                variant="primary"
                icon={<Copy size={14} />}
                onClick={() => {
                  copyInvite();
                  setCopied(true);
                }}
              >
                {copied ? "Copied" : "Copy link"}
              </Button>
            </div>
          </div>
        </>
      ) : (
        <p className="confirm__body" role="status">
          That link was used. Make a new one for the next person.
        </p>
      )}
      <div className="confirm__actions">
        <Button variant="secondary" icon={<RefreshCw size={14} />} onClick={() => void openInvite(true)}>
          New link
        </Button>
        <Button variant="ghost" onClick={closeInvite}>
          Done
        </Button>
      </div>
    </Dialog>
  );
}

const CARD_KEY = "platter.session-card.v1"; // gitleaks:allow (localStorage key name, not a secret)

function cardDismissed(): boolean {
  try {
    return localStorage.getItem(CARD_KEY) === "dismissed";
  } catch {
    return false;
  }
}

/**
 * "Listen along" in the queue panel (dismissable) and the device picker:
 * starts a listening session on what's playing. Hidden while in one.
 */
export function SessionCard({ dismissable = false }: { dismissable?: boolean }) {
  const role = useSocial((s) => s.listen.role);
  const [hidden, setHidden] = useState(() => dismissable && cardDismissed());
  if (!listenAlong.enabled || role !== "none" || hidden) return null;
  const dismiss = () => {
    setHidden(true);
    try {
      localStorage.setItem(CARD_KEY, "dismissed");
    } catch {
      /* this session only */
    }
  };
  return (
    <div className="session-card" role="group" aria-labelledby="session-card-title">
      <div className="session-card__head">
        <div id="session-card-title" className="session-card__title">
          Listen along
        </div>
        {dismissable && <IconButton size="sm" label="Dismiss" icon={<X size={16} />} onClick={dismiss} />}
      </div>
      <p className="session-card__text">
        Start a listening session and people with access to your Plex library can listen with you, from anywhere. They add songs, and can play, pause
        and skip if you let them.
      </p>
      <Button size="sm" variant="primary" icon={<UsersRound size={14} />} onClick={() => void startSession()}>
        Start a listening session
      </Button>
    </div>
  );
}
