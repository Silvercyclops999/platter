/**
 * Bring in music from another app (spec 27), laid out like a playlist page.
 *
 * The header says what came over in one sentence. Before a file: pick the
 * app for one line on getting the file, then drop it anywhere on the page
 * or choose it. The streaming app's playlists can come over at once through
 * a connected account while its data file is on the way.
 * After: liked songs found are liked and playlists made at once (missing
 * songs show greyed in those playlists until their album arrives), and the
 * albums to get are listed like an album's songs, most loved first, with
 * one button to get them. Without Lidarr that button opens a two-field
 * setup inline; the safety check runs on its own and only red stops it.
 */
import { AlertCircle, FileUp, Heart, RotateCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { LikedCover } from "../components/LikedCover";
import { Skeleton } from "../components/Skeleton";
import { ChipRow } from "../components/Tabs";
import { backend } from "../lib/backend";
import { failText } from "../lib/errorText";
import { BRING_CHOICES, LIKED_LIST_NAME, type BringChoice } from "../lib/importStrings";
import type { BringAlbum, BringStatus, ImportSourceId } from "../lib/importTypes";
import type { LidarrReport } from "../lib/types";
import { useDomId } from "../lib/domId";
import { usePhoneForm } from "../phone/form";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { ACCOUNT_SOURCE, AccountSetup, useAccount } from "./AccountConnect";
import { useTitleStep } from "./detail";
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

// ------------------------------------------------------------------ where from

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
      <p className="bring-text">
        Connect your account and your playlists come over now. Liked songs too, if you first select all of them in the app and drag them onto a new playlist
        named <strong>{LIKED_LIST_NAME}</strong>.
      </p>
      <AccountSetup />
      <Button variant="primary" disabled={!read || busy} onClick={adopt}>
        {busy ? "Bringing them over…" : "Bring these playlists over"}
      </Button>
    </div>
  );
}

/** The apps as chips; the picked one says how to get its file. */
function WhereFrom({ onBrought, title }: { onBrought: (s: BringStatus) => void; title: string }) {
  const [picked, setPicked] = useState<BringChoice["id"] | null>(null);
  const [now, setNow] = useState(false);
  const choice = BRING_CHOICES.find((c) => c.id === picked) ?? null;
  return (
    <section className="detail-section bring-section" aria-labelledby="bring-where">
      <h2 id="bring-where" className="shelf__title bring-section__title">
        {title}
      </h2>
      <ChipRow label="Where from">
        {BRING_CHOICES.map((c) => (
          <button
            key={c.id}
            type="button"
            className={`chip${c.id === picked ? " is-selected" : ""}`}
            aria-pressed={c.id === picked}
            onClick={() => setPicked(c.id === picked ? null : c.id)}
          >
            {c.name}
          </button>
        ))}
      </ChipRow>
      {choice && (
        <div className="bring-how">
          <p className="bring-text">{choice.how}</p>
          {(choice.link || choice.note) && (
            <p className="bring-text bring-text--muted">
              {choice.link && (
                <button
                  type="button"
                  className="bring-link"
                  onClick={() => void backend.importOpenLink(choice.link!.url).catch((e) => toast.error(failText("Couldn't open the page", e)))}
                >
                  {choice.link.label}
                </button>
              )}
              {choice.note ? ` ${choice.note}` : ""}
            </p>
          )}
          {picked === "streaming" &&
            (now ? (
              <PlaylistsNow onBrought={onBrought} />
            ) : (
              <Button variant="secondary" size="sm" onClick={() => setNow(true)}>
                Bring playlists now, the rest when your file arrives
              </Button>
            ))}
        </div>
      )}
    </section>
  );
}

// ------------------------------------------------------------------ the lists

function Playlists({ st }: { st: BringStatus }) {
  return (
    <section className="detail-section bring-section" aria-labelledby="bring-lists">
      <h2 id="bring-lists" className="shelf__title bring-section__title">
        Playlists
      </h2>
      <ul className="bring-rows" aria-label="Playlists brought over">
        {st.lists.map((l) => (
          <li key={l.importId}>
            <button
              type="button"
              className="bring-row bring-row--button"
              onClick={() => nav.go(l.playlistId && !l.gone ? { name: "playlist", id: l.playlistId } : { name: "import", view: `list:${l.importId}` })}
            >
              <Art path={l.thumb} size={40} kind="playlist" monogram={l.name} className="bring-row__art" />
              <span className="bring-row__text">
                <span className="bring-row__title truncate">{l.name}</span>
                <span className="bring-row__sub truncate">
                  {l.gone ? "You deleted it" : l.playlistId ? `${n(l.found)} of ${n(l.total)} here` : "Made once a song arrives"}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

// ------------------------------------------------------------------ Lidarr

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
        <input
          type="url"
          className="setting-text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="http://192.168.1.20:8686"
          spellCheck={false}
          autoComplete="off"
          required
        />
      </label>
      <label className="bring-field">
        <span className="bring-field__label">API key</span>
        <input
          type="password"
          className="setting-text"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder="Lidarr: Settings, General, API Key"
          spellCheck={false}
          autoComplete="off"
          required
        />
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

function AlbumRow({ a, i, onRetry }: { a: BringAlbum; i: number; onRetry: (key: string) => void }) {
  return (
    <li className={`bring-row bring-album is-${a.state}`} aria-label={`${a.album} by ${a.artist}, ${STATE_WORDS[a.state]}`}>
      <span className="bring-row__num tabular" aria-hidden="true">
        {i + 1}
      </span>
      <Art path={a.thumb} size={40} kind="album" monogram={a.album} className="bring-row__art" />
      <span className="bring-row__text">
        <span className="bring-row__title truncate">{a.album}</span>
        <span className="bring-row__sub truncate">{a.artist}</span>
      </span>
      <span className="bring-album__liked tabular">
        {a.liked > 0 ? (
          <>
            <Heart size={12} aria-hidden="true" fill="currentColor" /> {plural(a.liked, "liked song")}
          </>
        ) : (
          plural(a.songs, "song")
        )}
      </span>
      <span className="bring-album__state">
        <span className="bring-album__word">{STATE_WORDS[a.state]}</span>
        {a.state === "notFound" && a.reason && <span className="bring-album__reason">{a.reason.replace(/^Lidarr couldn't find it\.\s*/, "")}</span>}
      </span>
      {a.state === "notFound" ? (
        <Button size="sm" variant="ghost" icon={<RotateCw size={14} />} onClick={() => onRetry(a.key)}>
          Try again
        </Button>
      ) : (
        <span className="bring-album__end" />
      )}
    </li>
  );
}

function Albums({ st, onRetry }: { st: BringStatus; onRetry: (key: string) => void }) {
  const p = st.progress;
  return (
    <section className="detail-section bring-section" aria-labelledby="bring-albums">
      <h2 id="bring-albums" className="shelf__title bring-section__title">
        Albums to get
      </h2>
      {p.onTheWay > 0 ? (
        <p className="bring-text bring-text--muted">
          <span className="bring-progress" role="status" data-testid="bring-progress">
            {plural(p.onTheWay, "album")} on the way, {n(p.here)} here so far.
          </span>{" "}
          A few come each day so Lidarr isn't flooded, most loved first. Their liked songs are liked and slotted into your playlists as they arrive.
        </p>
      ) : (
        <p className="bring-text bring-text--muted">Most loved first. Their songs are liked and added to your playlists as they arrive.</p>
      )}
      {st.lidarr.practiceUntil && (
        <p className="bring-text bring-text--muted">
          Lidarr is in its practice week until {new Date(st.lidarr.practiceUntil * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}:
          Platter lists what it would get, then starts getting it.
        </p>
      )}
      <ul className="bring-rows bring-rows--albums" aria-label="Albums to get">
        {st.albums.map((a, i) => (
          <AlbumRow key={a.key} a={a} i={i} onRetry={onRetry} />
        ))}
      </ul>
    </section>
  );
}

// ------------------------------------------------------------------ page

const TITLE = "Bring music over";

export function BringPage() {
  const { st, setSt, err, load } = useBring();
  const { header, column, step, compact } = useTitleStep(TITLE);
  const phone = usePhoneForm((x) => x.phone);
  const [dragging, setDragging] = useState(false);
  const [reading, setReading] = useState(false);
  const [setup, setSetup] = useState(false);
  const [asking, setAsking] = useState(false);
  const depth = useRef(0);
  const brought = (s: BringStatus) => {
    setSt(s);
    if (!s.summary.sources.length) load();
  };
  const take = async (what: () => Promise<BringStatus>, failWhat: string) => {
    setReading(true);
    try {
      const s = await what();
      s.summary.sources.forEach(keepSource);
      importChanged();
      setSt(s);
    } catch (x) {
      toast.error(failText(failWhat, x, 400));
    } finally {
      setReading(false);
    }
  };
  const choose = async () => {
    let path: string | null;
    try {
      path = await backend.importChoose(false);
    } catch (e) {
      toast.error(failText("Couldn't open the picker", e));
      return;
    }
    if (path) await take(() => backend.bringRead(path), "Couldn't bring that file in");
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
    await take(async () => backend.bringReadDropped(new Uint8Array(await f.arrayBuffer()), f.name), `Couldn't bring ${f.name} in`);
  };
  const get = (keys: string[] | null) => {
    setAsking(true);
    backend.bringGet(keys).then(
      (s) => {
        setAsking(false);
        setSetup(false);
        setSt(s);
      },
      (e) => {
        setAsking(false);
        toast.error(failText("Couldn't ask for the albums", e));
      },
    );
  };
  const some = !!st && st.summary.sources.length > 0;
  const sent = st?.sent;
  const done: string[] = [];
  if (sent?.liked) done.push(`Liked ${plural(sent.liked, "song")}`);
  if (sent?.playlistsMade) done.push(`made ${plural(sent.playlistsMade, "playlist")}`);
  if (sent?.songsAdded && !sent.playlistsMade) done.push(`added ${plural(sent.songsAdded, "song")} to your playlists`);
  const toGet = st?.progress.toGet ?? 0;
  const unsure = st?.summary.unsure ?? 0;
  const headLines = (
    <>
      {!st && !err ? (
        <Skeleton width="40%" height={14} />
      ) : some ? (
        <>
          <p className="bring-head__line" data-testid="bring-sentence">
            {st!.summary.sentence}
          </p>
          {done.length > 0 && (
            <p className="bring-head__done detail-header__muted" role="status">
              {done.join(" and ")}.
            </p>
          )}
        </>
      ) : (
        <p className="bring-head__line">Your likes and playlists come over, and Lidarr gets what your library is missing.</p>
      )}
    </>
  );
  return (
    <div
      className={`detail bring-page${dragging ? " is-dragging" : ""}`}
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
      {phone ? (
        <header className="ph-playlist__head bring-phone-head">
          <div className="ph-playlist__cover">
            <LikedCover size={176} icon="bring" />
          </div>
          <h1 className="ph-playlist__title">{TITLE}</h1>
          {headLines}
        </header>
      ) : (
        <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
          <div className="detail-header__art">
            <LikedCover size={compact ? 130 : 232} icon="bring" />
          </div>
          <div ref={column} className="detail-header__text">
            <span className="detail-header__type">From another app</span>
            <h1 className="detail-header__title" data-step={step}>
              {TITLE}
            </h1>
            {headLines}
          </div>
        </header>
      )}
      <div className={phone ? "bring-phone-body" : "detail-body"}>
        <div className="action-row bring-actions">
          {some && toGet > 0 && !setup && (
            <Button variant="primary" size="lg" disabled={asking} onClick={() => (st!.lidarr.ready ? get(null) : setSetup(true))}>
              {asking ? "Asking…" : `Get the ${plural(toGet, "album")}`}
            </Button>
          )}
          <Button
            variant={some && toGet > 0 ? "secondary" : "primary"}
            size={some && toGet > 0 ? "md" : "lg"}
            icon={<FileUp size={16} />}
            disabled={reading}
            onClick={() => void choose()}
          >
            {reading ? "Reading and matching…" : "Choose file"}
          </Button>
          <span className="bring-actions__hint" data-testid="bring-drop">
            or drop it anywhere on this page
          </span>
        </div>
        {setup && <LidarrInline onReady={() => get(null)} />}
        {err && (
          <p className="bring-problem" role="alert">
            <AlertCircle size={16} aria-hidden="true" /> {err}
          </p>
        )}
        {unsure > 0 && (
          <p className="bring-text bring-text--muted bring-unsure">
            {plural(unsure, "song")} might be in your library under another version.{" "}
            <button type="button" className="bring-link" onClick={() => nav.go({ name: "import" })}>
              Check {unsure === 1 ? "it" : "them"}
            </button>
          </p>
        )}
        {st && !some && <WhereFrom onBrought={brought} title="Where's your music now?" />}
        {some && st.albums.length > 0 && <Albums st={st} onRetry={(k) => get([k])} />}
        {some && st.lists.length > 0 && <Playlists st={st} />}
        {some && <WhereFrom onBrought={brought} title="Bring in another file" />}
      </div>
      {(dragging || reading) && (
        <div className="bring-overlay" role="status">
          <FileUp size={40} aria-hidden="true" />
          <span>{reading ? "Reading and matching to your library…" : "Drop to bring it in"}</span>
        </div>
      )}
    </div>
  );
}
