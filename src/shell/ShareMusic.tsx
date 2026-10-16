import { Check, Copy, EarOff, LinkIcon, LockKeyhole, Server, X } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Button, IconButton } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { QrCode } from "../components/QrCode";
import { Switch } from "../components/Switch";
import type { ListenPerson, ListenView, ReceivedInvite, ShareStatus, SharingLibrary } from "../lib/types";
import {
  acceptInvite,
  closeShareDialog,
  copyShareLink,
  defaultOptions,
  dismissInvite,
  letThemListen,
  makeShareLink,
  offerServer,
  shareWith,
  statusText,
  unreleasedIn,
  useSharing,
} from "../stores/sharing";
import { updateSettings, useSettings } from "../stores/settings";
import { useSocial } from "../stores/social";
import { PersonFace } from "./ListenStrip";
import "./share-music.css";

/** "4 unreleased albums", "Nothing unreleased", "Not read yet". */
export function unreleasedLabel(n: number | null): string {
  if (n === null) return "Not read by Platter yet";
  if (n === 0) return "Nothing unreleased";
  return n === 1 ? "1 unreleased album" : `${n} unreleased albums`;
}

/** What the host sees about one guest's offer. */
export function offerText(p: ListenPerson): string | null {
  const o = p.offer;
  if (!o) return null;
  switch (o.state) {
    case "asking":
    case "inviting":
      return `Inviting ${p.name}…`;
    case "invited":
      return `Invited. ${p.name} accepts in their Platter.`;
    case "alreadyShared":
      return `${p.name} already has access. They can accept in Platter if they haven't.`;
    case "noInvites":
      return `${p.name}'s Platter doesn't take invites. Share from Settings, Sharing with their Plex username.`;
    case "failed":
      return o.message;
  }
}

/**
 * Host: "<name> can't hear this" with "Let them listen from my server",
 * one card per guest who can't hear the song (or whose invite is going out).
 */
export function CantHearCallout({ v }: { v: ListenView }) {
  if (v.role !== "host" || v.status !== "live" || !v.offerServer) return null;
  const people = v.people.filter((p) => !p.host && (p.cantHear || (p.offer && p.offer.state !== "invited" && p.offer.state !== "alreadyShared")));
  if (!people.length) return null;
  return (
    <div className="share-callouts" role="group" aria-label="People who can't hear this">
      {people.slice(0, 3).map((p) => {
        const said = offerText(p);
        const busy = p.offer?.state === "asking" || p.offer?.state === "inviting";
        const done = p.offer?.state === "invited" || p.offer?.state === "alreadyShared";
        return (
          <div key={p.id} className={`share-callout ${done ? "share-callout--done" : ""}`} role="status">
            <span className="share-callout__face">
              <PersonFace person={p} size={32} />
              <span className="share-callout__glyph" aria-hidden="true">
                <EarOff size={12} />
              </span>
            </span>
            <span className="share-callout__text">
              <span className="share-callout__title truncate">{p.cantHear ? `${p.name} can't hear this` : p.name}</span>
              <span className="share-callout__sub">{said ?? "It isn't on any server they can reach."}</span>
            </span>
            {!done && (
              <Button size="sm" variant="primary" icon={<Server size={14} />} disabled={busy} onClick={() => void letThemListen(p.id, p.name)}>
                {p.offer?.state === "failed" || p.offer?.state === "noInvites" ? "Try again" : "Let them listen from my server"}
              </Button>
            )}
            {done && <Check size={16} className="share-callout__ok" aria-label="Invited" />}
          </div>
        );
      })}
    </div>
  );
}

/** Guest (or anyone): a server invite waiting, accepted with one click. */
export function InviteCallout({ invite, compact = false }: { invite: ReceivedInvite; compact?: boolean }) {
  const busy = useSharing((s) => s.accepting.has(invite.id));
  const host = useSocial((s) => (s.listen.role === "guest" ? s.listen.hostName : null));
  const inSession = host === invite.from;
  return (
    <div className={`share-callout share-callout--invite ${compact ? "share-callout--compact" : ""}`} role="group" aria-label={`Invite from ${invite.from}`}>
      <span className="share-callout__face">
        <PersonFace person={{ name: invite.from, avatar: null }} size={32} />
        <span className="share-callout__glyph share-callout__glyph--server" aria-hidden="true">
          <Server size={12} />
        </span>
      </span>
      <span className="share-callout__text">
        <span className="share-callout__title truncate">{invite.from} shared their music with you</span>
        <span className="share-callout__sub">
          {inSession ? `Accept to hear ${invite.from}'s copy of this song and the rest of the session.` : `${invite.servers[0] ?? "Their server"} joins your library. Plex may email you about it too.`}
        </span>
      </span>
      <Button size="sm" variant="primary" disabled={busy} onClick={() => void acceptInvite(invite)}>
        {busy ? "Adding…" : "Accept"}
      </Button>
      <IconButton size="sm" label="Not now" icon={<X size={16} />} onClick={() => dismissInvite(invite.id)} />
    </div>
  );
}

/** The guest's card in a session: the host's invite. */
export function SessionInviteCard() {
  const v = useSocial((s) => s.listen);
  const invites = useSharing((s) => s.invites);
  const accept = useSettings((s) => s.settings?.shareAcceptInvites ?? true);
  if (!accept || v.role !== "guest" || v.status !== "live") return null;
  const mine = invites.find((i) => i.from === v.hostName) ?? (v.missing ? invites[0] : undefined);
  if (!mine) return null;
  return (
    <div className="share-callouts">
      <InviteCallout invite={mine} />
    </div>
  );
}

function LibraryPick({ lib, on, onChange }: { lib: SharingLibrary; on: boolean; onChange: (on: boolean) => void }) {
  const has = lib.unreleased === null || lib.unreleased > 0;
  return (
    <label className={`share-lib ${on ? "is-on" : ""}`}>
      <input type="checkbox" className="share-lib__box" checked={on} onChange={(e) => onChange(e.target.checked)} />
      <span className="share-lib__disc" aria-hidden="true" />
      <span className="share-lib__text">
        <span className="share-lib__title truncate">{lib.title}</span>
        <span className={`share-lib__sub ${has ? "share-lib__sub--vault" : ""}`}>
          {has && <LockKeyhole size={11} aria-hidden="true" />}
          {unreleasedLabel(lib.unreleased)}
        </span>
      </span>
    </label>
  );
}

/**
 * Share my music: with a guest in your session ("Let them listen from my
 * server") or anyone by Plex username, email or a link. Music libraries
 * only; says how many unreleased albums the choice holds before sharing.
 */
export function ShareMusicDialog() {
  const target = useSharing((s) => s.dialog);
  const overview = useSharing((s) => s.overview);
  const link = useSharing((s) => s.link);
  const [libs, setLibs] = useState<string[]>([]);
  const [hide, setHide] = useState(true);
  const [downloads, setDownloads] = useState(false);
  const [who, setWho] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ShareStatus | null>(null);
  const field = useRef<HTMLInputElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  const ids = { hide: useId(), downloads: useId(), who: useId() };
  const open = target !== null;
  useEffect(() => {
    if (!open) return;
    const d = defaultOptions();
    setLibs(d.libraries);
    setHide(d.hideUnreleased);
    setDownloads(d.allowDownloads);
    setWho("");
    setResult(null);
  }, [open, target]);
  // Nothing preselected: with one music library, that's the obvious one.
  useEffect(() => {
    if (open && overview && libs.length === 0 && overview.libraries.length === 1) setLibs([overview.libraries[0].id]);
  }, [open, overview]); // eslint-disable-line react-hooks/exhaustive-deps
  const count = useMemo(() => unreleasedIn(overview, libs), [overview, libs]);
  if (!target) return null;
  const guest = target.kind === "guest" ? target : null;
  const title = guest ? `Let ${guest.name} listen from your server` : "Share my music";
  const noPass = hide && overview?.plexPass === false && (count === null || count > 0);
  const pass = result?.status === "needsPlexPass" ? result : null;
  const withUnreleased = overview?.libraries.filter((l) => libs.includes(l.id) && l.unreleased !== 0) ?? [];
  const others = overview?.libraries.filter((l) => libs.includes(l.id) && l.unreleased === 0) ?? [];
  const canShare = libs.length > 0 && !busy && (guest || who.trim().length > 0) && !!overview?.owner;
  const toggle = (id: string, on: boolean) => {
    setResult(null);
    setLibs((l) => (on ? [...new Set([...l, id])] : l.filter((x) => x !== id)));
  };

  const go = async (chosen = libs) => {
    setBusy(true);
    setResult(null);
    try {
      const options = { libraries: chosen, hideUnreleased: hide, allowDownloads: downloads };
      if (guest) {
        // The core shares the default libraries; the one click from now on uses these too.
        await updateSettings({ shareDefaultLibraries: chosen, shareHideUnreleased: hide, shareAllowDownloads: downloads });
        if (noPass && chosen.some((id) => withUnreleased.some((l) => l.id === id))) {
          setResult({ status: "needsPlexPass", withUnreleased: withUnreleased.map((l) => l.title), others: others.map((l) => l.id) });
          return;
        }
        await offerServer(guest.personId);
        closeShareDialog();
        return;
      }
      const st = await shareWith(who, options);
      if (!st) return;
      if (st.status === "invited" || st.status === "alreadyShared") closeShareDialog();
      else setResult(st);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onClose={closeShareDialog} label={title} className="confirm share-dialog" initialFocus={guest ? first : field}>
      <h2 className="confirm__title">{title}</h2>
      <p className="confirm__body">
        {guest
          ? `${guest.name} gets an invite to your Plex server, for the music libraries you pick here and nothing else. They accept in Platter and hear your copy of every song.`
          : "They get an invite to your Plex server, for the music libraries you pick and nothing else. Plex may email them too."}
      </p>
      {!guest && (
        <div className="share-dialog__who">
          <label htmlFor={ids.who} className="share-dialog__label">
            Plex username or email
          </label>
          <input
            id={ids.who}
            ref={field}
            className="setting-text"
            value={who}
            maxLength={200}
            autoComplete="off"
            spellCheck={false}
            placeholder="robin_listens"
            onChange={(e) => {
              setWho(e.target.value);
              setResult(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && canShare) void go();
            }}
          />
        </div>
      )}
      <div className="share-dialog__label" id={`${ids.who}-libs`}>
        Libraries
      </div>
      {!overview ? (
        <p className="share-dialog__note" role="status">
          Reading your libraries…
        </p>
      ) : !overview.owner ? (
        <p className="share-dialog__note" role="status">
          Only the server's owner can share it. {overview.serverName ?? "This server"} is shared with you.
        </p>
      ) : (
        <div className="share-libs" role="group" aria-labelledby={`${ids.who}-libs`}>
          {overview.libraries.map((l) => (
            <LibraryPick key={l.id} lib={l} on={libs.includes(l.id)} onChange={(on) => toggle(l.id, on)} />
          ))}
        </div>
      )}
      {overview?.owner && libs.length > 0 && (
        <div className={`share-vault ${count ? "share-vault--has" : ""}`}>
          <LockKeyhole size={16} aria-hidden="true" className="share-vault__icon" />
          <div className="share-vault__body">
            <div className="share-vault__line" role="status">
              {count === null
                ? "Platter hasn't read every library you picked, so it can't count their unreleased albums."
                : count === 0
                  ? "Nothing unreleased in what you picked."
                  : `What you picked holds ${count === 1 ? "1 unreleased album" : `${count} unreleased albums`}.`}
            </div>
            <div className="share-vault__row">
              <span id={ids.hide}>Hide my unreleased music from people I share with</span>
              <Switch checked={hide} onChange={(v) => (setHide(v), setResult(null))} labelledBy={ids.hide} />
            </div>
            {hide && count !== 0 && !noPass && <p className="share-vault__hint">Platter labels them in Plex and the share leaves that label out, so they never show up.</p>}
            {(noPass || pass) && (
              <div className="share-vault__stop" role="alert">
                <p>Keeping unreleased music out needs Plex Pass on your account, and it doesn't have it. Nothing is shared until you choose.</p>
                <div className="share-vault__actions">
                  {others.length > 0 && (
                    <Button size="sm" variant="secondary" disabled={busy} onClick={() => void go(others.map((l) => l.id))}>
                      Share without {withUnreleased.map((l) => l.title).join(" and ")}
                    </Button>
                  )}
                  <Button size="sm" variant="ghost" onClick={closeShareDialog}>
                    Cancel
                  </Button>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
      {overview?.owner && (
        <div className="share-dialog__opts">
          <div className="share-vault__row">
            <span id={ids.downloads}>Allow downloads</span>
            <Switch checked={downloads} onChange={setDownloads} labelledBy={ids.downloads} />
          </div>
          {guest && <p className="share-dialog__note">These become your sharing defaults, so next time it's one click.</p>}
        </div>
      )}
      {result && result.status !== "needsPlexPass" && (
        <p className="share-dialog__result" role="alert">
          {statusText(result, who.trim() || guest?.name || "They")}
        </p>
      )}
      {link && !guest && (
        <div className="invite-dialog__code share-dialog__link">
          {link.qr && <QrCode qr={link.qr} label="QR code for the share link" className="invite-dialog__qr" />}
          <div className="invite-dialog__link">
            <input className="setting-text" readOnly value={link.link} aria-label="Share link" onFocus={(e) => e.currentTarget.select()} />
            <Button size="sm" variant="secondary" icon={<Copy size={14} />} onClick={copyShareLink}>
              Copy link
            </Button>
            <span className="share-dialog__note">Works for one person, once. Keep Platter open until they've used it.</span>
          </div>
        </div>
      )}
      <div className="confirm__actions">
        {!guest && overview?.owner && (
          <Button variant="ghost" icon={<LinkIcon size={14} />} disabled={!libs.length || (noPass && (count ?? 1) > 0)} onClick={() => void makeShareLink({ libraries: libs, hideUnreleased: hide, allowDownloads: downloads })}>
            {link ? "New link" : "Make a link instead"}
          </Button>
        )}
        <Button variant="ghost" onClick={closeShareDialog}>
          Cancel
        </Button>
        <Button ref={first} variant="primary" disabled={!canShare || (noPass && others.length === 0)} onClick={() => void go()}>
          {busy ? "Sharing…" : guest ? "Invite" : "Share"}
        </Button>
      </div>
    </Dialog>
  );
}
