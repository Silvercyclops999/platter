import { Link, UsersRound, X } from "lucide-react";
import { useState } from "react";
import { Button, IconButton } from "../components/Button";
import { copyInvite, endSession, startSession, useSocial } from "../stores/social";
import "./social.css";

/** "Alex", "Alex and Sam", "Alex, Sam and 2 more". */
export function namesLine(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  if (names.length === 3) return `${names[0]}, ${names[1]} and ${names[2]}`;
  return `${names[0]}, ${names[1]} and ${names.length - 2} more`;
}

/** The strip above the player bar while in a listening session: who's in it, and how to leave. */
export function ListenStrip() {
  const v = useSocial((s) => s.listen);
  if (v.role === "none") return null;
  const host = v.hostName ?? "the host";
  let text: string;
  if (v.role === "host") {
    text = v.listeners.length ? `Listening session: ${namesLine(v.listeners)} listening along` : "Listening session: waiting for people to join";
  } else if (v.status === "hostGone") {
    text = `${host} stopped answering`;
  } else if (v.status === "ended") {
    text = `${host} ended the listening session`;
  } else if (v.status === "connecting") {
    text = `Joining ${host}'s listening session…`;
  } else {
    const others = v.listeners.filter((n) => n !== v.hostName);
    text = others.length ? `Listening along with ${host}, ${namesLine(others)}` : `Listening along with ${host}`;
  }
  const stopped = v.status === "hostGone" || v.status === "ended";
  return (
    <div className={`listen-strip ${stopped ? "listen-strip--stopped" : ""}`} role="region" aria-label="Listening session">
      <UsersRound size={14} aria-hidden="true" />
      <span className="listen-strip__text truncate" role="status">
        {text}
      </span>
      {v.role === "host" && v.invite && (
        <button type="button" className="listen-strip__btn" onClick={copyInvite}>
          <Link size={14} aria-hidden="true" />
          Copy invite link
        </button>
      )}
      <button type="button" className="listen-strip__btn" onClick={() => void endSession()}>
        {v.role === "host" ? "End session" : stopped ? "Close" : "Leave"}
      </button>
    </div>
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
  if (role !== "none" || hidden) return null;
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
      <p className="session-card__text">Start a listening session and people on your Plex server can listen with you. They can add songs too, if you let them.</p>
      <Button size="sm" variant="primary" icon={<UsersRound size={14} />} onClick={() => void startSession()}>
        Start a listening session
      </Button>
    </div>
  );
}
