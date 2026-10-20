/**
 * Bring in music from another app (spec 27): one page, three steps.
 *
 * 1. Where from: a big choice per app with one line on getting the file.
 *    A file dropped anywhere on the page (or picked) is worked out by the
 *    core. The streaming app's playlists can come over at once through a
 *    connected account while its data file is on the way.
 * 2. What you already have: one sentence and two numbers. Liked songs found
 *    are liked and playlists made at once; missing songs show greyed in
 *    those playlists until their album arrives.
 * 3. Get the rest: one button. Without Lidarr it opens a two-field setup
 *    inline; the safety check runs on its own and only red stops anything.
 */
import { AlertCircle, Check, Clock, FileUp, Heart, ListMusic, Loader2, Music } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { failText } from "../lib/errorText";
import { BRING_CHOICES, LIKED_LIST_NAME, type BringChoice } from "../lib/importStrings";
import type { BringAlbum, BringStatus, ImportSourceId } from "../lib/importTypes";
import type { LidarrReport } from "../lib/types";
import { useDomId } from "../lib/domId";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { ACCOUNT_SOURCE, AccountSetup, useAccount } from "./AccountConnect";
import { errText, importChanged } from "./importShared";
import "./bring.css";

const n = (v: number) => v.toLocaleString();
const plural = (v: number, one: string, many = `${one}s`) => `${n(v)} ${v === 1 ? one : many}`;

/** Turn a source on in Settings > Library import, so re-imports and the report find it. */
function keepSource(source: ImportSourceId) {
  const on = useSettings.getState().settings?.importSources ?? [];
  if (!on.includes(source)) void updateSettings({ importSources: [...on, source] });
}

/** The page's state, loaded once and after every step. */
function useBring() {
  const [st, setSt] = useState<BringStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const load = useCallback(() => {
    backend.bringStatus().then(
      (s) => {
        setSt(s);
        setErr(null);
      },
      (e) => setErr(errText(e) || "Couldn't load what was brought over"),
    );
  }, []);
  useEffect(() => {
    load();
    // Albums arriving change the numbers.
    const un = backend.listen("library://changed", () => load());
    return () => void un.then((f) => f());
  }, [load]);
  // While MusicBrainz picks albums, look again every few seconds.
  const picking = !!st?.albums.some((a) => a.state === "picking");
  useEffect(() => {
    if (!picking) return;
    const t = setInterval(load, 4000);
    return () => clearInterval(t);
  }, [picking, load]);
  return { st, setSt, err, load };
}

// ------------------------------------------------------------------ step 1

function Choice({ c, picked, onPick }: { c: BringChoice; picked: boolean; onPick: () => void }) {
  return (
    <button type="button" className={`bring-choice${picked ? " is-picked" : ""}`} aria-pressed={picked} onClick={onPick}>
      <span className="bring-choice__name">{c.name}</span>
      <span className="bring-choice__how">{c.how}</span>
    </button>
  );
}

/** "Bring playlists now, the rest when your file arrives": the connected account. */
function PlaylistsNow({ onBrought }: { onBrought: (s: BringStatus) => void }) {
  const a = useAccount();
  const [busy, setBusy] = useState(false);
  useEffect(() => keepSource(ACCOUNT_SOURCE), []);
  const read = !!a.status?.lastRead;
  const adopt = () => {
    setBusy(true);
    backend.bringAdopt(ACCOUNT_SOURCE).then(
      (s) => {
        setBusy(false);
        importChanged();
        onBrought(s);
      },
      (e) => {
        setBusy(false);
        toast.error(failText("Couldn't bring your playlists over", e));
      },
    );
  };
  return (
    <div className="bring-now" data-testid="bring-playlists-now">
      <p className="bring-now__lead">
        Connect your account and your playlists come over now. Liked songs too, if you first select all of them in the app and drag them
        onto a new playlist named <strong>{LIKED_LIST_NAME}</strong>.
      </p>
      <AccountSetup />
      <Button variant="primary" disabled={!read || busy} onClick={adopt}>
        {busy ? "Bringing them over…" : "Bring these playlists over"}
      </Button>
    </div>
  );
}

function StepWhere({ onBrought, compact }: { onBrought: (s: BringStatus) => void; compact: boolean }) {
  const [picked, setPicked] = useState<BringChoice["id"] | null>(null);
  const [now, setNow] = useState(false);
  const [busy, setBusy] = useState(false);
  const choice = BRING_CHOICES.find((c) => c.id === picked) ?? null;
  const pick = async () => {
    let path: string | null;
    try {
      path = await backend.importChoose(false);
    } catch (e) {
      toast.error(failText("Couldn't open the picker", e));
      return;
    }
    if (!path) return;
    setBusy(true);
    try {
      const s = await backend.bringRead(path);
      s.summary.sources.forEach(keepSource);
      importChanged();
      onBrought(s);
    } catch (e) {
      toast.error(failText("Couldn't bring that file in", e, 400));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="bring-step" aria-labelledby="bring-where">
      <h2 id="bring-where" className="bring-step__title">
        {!compact && (
          <span className="bring-step__num" aria-hidden="true">
            1
          </span>
        )}
        {compact ? "Bring in another file" : "Where is your music now?"}
      </h2>
      <div className="bring-choices" role="group" aria-label="Where from">
        {BRING_CHOICES.map((c) => (
          <Choice key={c.id} c={c} picked={c.id === picked} onPick={() => setPicked(c.id === picked ? null : c.id)} />
        ))}
      </div>
      {choice?.link && (
        <p className="bring-step__note">
          <button type="button" className="bring-link" onClick={() => void backend.importOpenLink(choice.link!.url).catch((e) => toast.error(failText("Couldn't open the page", e)))}>
            {choice.link.label}
          </button>
          {choice.note ? ` ${choice.note}` : ""}
        </p>
      )}
      <div className="bring-drop" data-testid="bring-drop">
        <FileUp size={28} aria-hidden="true" />
        <p>
          <strong>Drop the zip or file anywhere on this page</strong>
          <span>Platter works out what it is.</span>
        </p>
        <Button onClick={() => void pick()} disabled={busy}>
          {busy ? "Reading and matching…" : "Choose file"}
        </Button>
      </div>
      {picked === "streaming" &&
        (now ? (
          <PlaylistsNow onBrought={onBrought} />
        ) : (
          <Button variant="ghost" className="bring-now__open" onClick={() => setNow(true)}>
            Bring playlists now, the rest when your file arrives
          </Button>
        ))}
    </section>
  );
}

// ------------------------------------------------------------------ step 2

function StepHave({ st }: { st: BringStatus }) {
  const s = st.summary;
  const liked = s.liked.total > 0;
  const found = liked ? s.liked.found : s.songs.found;
  const sent = st.sent;
  const done: string[] = [];
  if (sent.liked) done.push(`Liked ${plural(sent.liked, "song")}`);
  if (sent.playlistsMade) done.push(`made ${plural(sent.playlistsMade, "playlist")}`);
  if (sent.songsAdded && !sent.playlistsMade) done.push(`added ${plural(sent.songsAdded, "song")} to your playlists`);
  return (
    <section className="bring-step" aria-labelledby="bring-have">
      <h2 id="bring-have" className="bring-step__title">
        <span className="bring-step__num" aria-hidden="true">2</span>
        What you already have
      </h2>
      <p className="bring-sentence" data-testid="bring-sentence">
        {s.sentence}
      </p>
      <div className="bring-numbers">
        <div className="bring-number">
          <span className="bring-number__value">{n(found)}</span>
          <span className="bring-number__label">{liked ? "liked songs here" : "songs here"}</span>
        </div>
        <div className="bring-number">
          <span className="bring-number__value">{n(s.missingAlbums)}</span>
          <span className="bring-number__label">{s.missingAlbums === 1 ? "album to get" : "albums to get"}</span>
        </div>
      </div>
      {done.length > 0 && (
        <p className="bring-done" role="status">
          <Check size={16} aria-hidden="true" /> {done.join(" and ")}.
        </p>
      )}
      {s.unsure > 0 && (
        <p className="bring-step__note">
          {plural(s.unsure, "song")} might be in your library under another version.{" "}
          <button type="button" className="bring-link" onClick={() => nav.go({ name: "import" })}>
            Check {s.unsure === 1 ? "it" : "them"}
          </button>
        </p>
      )}
      {st.lists.length > 0 && (
        <ul className="bring-lists" aria-label="Playlists brought over">
          {st.lists.map((l) => (
            <li key={l.importId}>
              <button
                type="button"
                className="bring-list"
                onClick={() => nav.go(l.playlistId && !l.gone ? { name: "playlist", id: l.playlistId } : { name: "import", view: `list:${l.importId}` })}
              >
                <ListMusic size={18} aria-hidden="true" />
                <span className="bring-list__name truncate">{l.name}</span>
                <span className="bring-list__count">
                  {l.gone ? "You deleted it" : l.playlistId ? `${n(l.found)} of ${n(l.total)} here` : "Made once a song arrives"}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

// ------------------------------------------------------------------ step 3

/** The two-field Lidarr setup, inline. The safety check runs on its own. */
function LidarrInline({ onReady }: { onReady: () => void }) {
  const id = useDomId();
  const saved = useSettings((s) => s.settings?.lidarrUrl ?? "");
  const [url, setUrl] = useState(saved);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [red, setRed] = useState<LidarrReport["findings"]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (saved) return;
    backend.bringLidarrGuesses().then(
      (g) => setUrl((u) => u || g[0] || ""),
      () => {},
    );
  }, [saved]);
  const connect = async () => {
    setBusy(true);
    setError(null);
    setRed([]);
    try {
      const address = url.trim().replace(/\/+$/, "");
      if (address !== saved) await updateSettings({ lidarrUrl: address });
      await backend.lidarrKeySet(key.trim());
      const report = await backend.lidarrCheck();
      const stops = report.findings.filter((f) => f.level === "red" && !f.protectedOnly);
      if (stops.length) {
        setRed(stops);
      } else {
        setKey("");
        onReady();
      }
    } catch (e) {
      setError(errText(e) || "Couldn't reach Lidarr");
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="bring-lidarr"
      aria-labelledby={`${id}-h`}
      onSubmit={(e) => {
        e.preventDefault();
        void connect();
      }}
    >
      <p id={`${id}-h`} className="bring-lidarr__lead">
        Platter gets albums through Lidarr on your server. Two things and it's set up.
      </p>
      <label className="bring-field">
        <span className="bring-field__label">Address</span>
        <input type="url" className="setting-text" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="http://192.168.1.20:8686" spellCheck={false} autoComplete="off" required />
      </label>
      <label className="bring-field">
        <span className="bring-field__label">API key</span>
        <input type="password" className="setting-text" value={key} onChange={(e) => setKey(e.target.value)} placeholder="Lidarr: Settings, General, API Key" spellCheck={false} autoComplete="off" required />
      </label>
      <Button type="submit" variant="primary" disabled={busy || !url.trim() || !key.trim()}>
        {busy ? "Checking Lidarr…" : "Connect"}
      </Button>
      {error && (
        <p className="bring-problem" role="alert">
          <AlertCircle size={16} aria-hidden="true" /> {error}
        </p>
      )}
      {red.length > 0 && (
        <div className="bring-problem" role="alert">
          <AlertCircle size={16} aria-hidden="true" />
          <div>
            <p>Lidarr needs a change before Platter asks it for anything:</p>
            <ul>
              {red.map((f) => (
                <li key={f.id}>
                  <strong>{f.title}</strong> {f.fix ?? f.detail}
                </li>
              ))}
            </ul>
            <button type="button" className="bring-link" onClick={() => nav.go({ name: "settings", page: "lidarr" })}>
              Open Lidarr settings
            </button>
          </div>
        </div>
      )}
    </form>
  );
}

const STATE_WORDS: Record<BringAlbum["state"], string> = {
  missing: "Not asked for yet",
  picking: "Finding it",
  asked: "On the way",
  notFound: "Lidarr couldn't find it",
  here: "Here",
};

function AlbumRow({ a, onRetry }: { a: BringAlbum; onRetry: (key: string) => void }) {
  const Icon = a.state === "here" ? Check : a.state === "notFound" ? AlertCircle : a.state === "picking" ? Loader2 : a.state === "asked" ? Clock : Music;
  return (
    <li className={`bring-album is-${a.state}`} aria-label={`${a.album} by ${a.artist}, ${STATE_WORDS[a.state]}`}>
      <span className="bring-album__text">
        <span className="bring-album__title truncate">{a.album}</span>
        <span className="bring-album__meta truncate">
          {a.artist}
          {a.liked > 0 && (
            <>
              {" • "}
              <Heart size={12} aria-hidden="true" className="bring-album__heart" /> {plural(a.liked, "liked song")}
            </>
          )}
          {a.liked === 0 && a.songs > 0 && ` • ${plural(a.songs, "song")}`}
        </span>
        {a.state === "notFound" && a.reason && <span className="bring-album__reason">{a.reason}</span>}
      </span>
      <span className="bring-album__state">
        <Icon size={14} aria-hidden="true" />
        {STATE_WORDS[a.state]}
      </span>
      {a.state === "notFound" && (
        <Button size="sm" variant="ghost" onClick={() => onRetry(a.key)}>
          Try again
        </Button>
      )}
    </li>
  );
}

function StepGet({ st, setSt }: { st: BringStatus; setSt: (s: BringStatus) => void }) {
  const [setup, setSetup] = useState(false);
  const [busy, setBusy] = useState(false);
  const p = st.progress;
  const get = (keys: string[] | null) => {
    setBusy(true);
    backend.bringGet(keys).then(
      (s) => {
        setBusy(false);
        setSetup(false);
        setSt(s);
      },
      (e) => {
        setBusy(false);
        toast.error(failText("Couldn't ask for the albums", e));
      },
    );
  };
  const onGet = () => (st.lidarr.ready ? get(null) : setSetup(true));
  const asked = p.onTheWay > 0;
  return (
    <section className="bring-step" aria-labelledby="bring-get">
      <h2 id="bring-get" className="bring-step__title">
        <span className="bring-step__num" aria-hidden="true">3</span>
        Get the rest
      </h2>
      {asked && (
        <p className="bring-progress" role="status" data-testid="bring-progress">
          {plural(p.onTheWay, "album")} on the way, {n(p.here)} here so far.
        </p>
      )}
      {asked && <p className="bring-step__note">A few come each day so Lidarr isn't flooded, most loved first. Their liked songs are liked and slotted into your playlists as they arrive.</p>}
      {st.lidarr.practiceUntil && (
        <p className="bring-step__note">
          Lidarr is in its practice week until {new Date(st.lidarr.practiceUntil * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}: Platter lists what it would get, then starts getting it.
        </p>
      )}
      {p.toGet > 0 && !setup && (
        <Button variant="primary" size="lg" disabled={busy} onClick={onGet}>
          {busy ? "Asking…" : `Get the ${plural(p.toGet, "album")}`}
        </Button>
      )}
      {setup && <LidarrInline onReady={() => get(null)} />}
      {st.albums.length > 0 && (
        <ul className="bring-albums" aria-label="Albums to get">
          {st.albums.map((a) => (
            <AlbumRow key={a.key} a={a} onRetry={(k) => get([k])} />
          ))}
        </ul>
      )}
    </section>
  );
}

// ------------------------------------------------------------------ page

export function BringPage() {
  const { st, setSt, err, load } = useBring();
  const [dragging, setDragging] = useState(false);
  const [reading, setReading] = useState(false);
  const depth = useRef(0);
  const brought = (s: BringStatus) => {
    setSt(s);
    if (!s.summary.sources.length) load();
  };
  const onDrop = async (e: DragEvent) => {
    e.preventDefault();
    depth.current = 0;
    setDragging(false);
    const files = [...e.dataTransfer.files];
    if (!files.length) return;
    if (files.length > 1) {
      toast.error("Drop one file at a time: the zip, the library file or one list.");
      return;
    }
    const f = files[0];
    setReading(true);
    try {
      const s = await backend.bringReadDropped(new Uint8Array(await f.arrayBuffer()), f.name);
      s.summary.sources.forEach(keepSource);
      importChanged();
      setSt(s);
    } catch (x) {
      toast.error(failText(`Couldn't bring ${f.name} in`, x, 400));
    } finally {
      setReading(false);
    }
  };
  const some = !!st && st.summary.sources.length > 0;
  return (
    <div
      className={`page-pad bring-page${dragging ? " is-dragging" : ""}`}
      data-testid="bring-page"
      onDragEnter={(e) => {
        if (!e.dataTransfer.types.includes("Files")) return;
        depth.current += 1;
        setDragging(true);
      }}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) e.preventDefault();
      }}
      onDragLeave={() => {
        depth.current = Math.max(0, depth.current - 1);
        if (!depth.current) setDragging(false);
      }}
      onDrop={(e) => void onDrop(e)}
    >
      <h1 className="page-title">Bring in music from another app</h1>
      <p className="bring-page__sub">Your likes and playlists come over, and Lidarr gets what your library is missing.</p>
      {err && (
        <p className="bring-problem" role="alert">
          {err}
        </p>
      )}
      {!st && !err ? (
        <div aria-busy="true">
          <Skeleton width="100%" height={220} radius="card" />
        </div>
      ) : (
        st && (
          <>
            {some && <StepHave st={st} />}
            {some && (st.albums.length > 0 || st.progress.onTheWay > 0) && <StepGet st={st} setSt={setSt} />}
            <StepWhere onBrought={brought} compact={some} />
          </>
        )
      )}
      {(dragging || reading) && (
        <div className="bring-overlay" role="status">
          <FileUp size={40} aria-hidden="true" />
          <span>{reading ? "Reading and matching to your library…" : "Drop to bring it in"}</span>
        </div>
      )}
    </div>
  );
}
