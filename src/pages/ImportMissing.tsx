/**
 * The Missing list: every imported song that isn't in the library, the most
 * wanted first, with store searches. When Lidarr is set up and its check is
 * green, each row also gets Request: Platter finds the official album the
 * song comes from, then the usual request preview and confirm.
 */
import { Clock, Download } from "lucide-react";
import { Suspense, lazy, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { SkeletonLines } from "../components/Skeleton";
import { VirtualRows } from "../components/VirtualRows";
import { backend } from "../lib/backend";
import { STORE_LINKS } from "../lib/importStrings";
import type { MissingSong } from "../lib/importTypes";
import type { LidarrRequestInput, LidarrRequestState, MissingAlbum } from "../lib/types";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import { IMPORT_ROW_H } from "./ImportListView";
import { clearMissingFocus, errText, missingFocusKey, missingKey } from "./importShared";
import { STATE_LABEL, useLidarrReady } from "./lidarrShared";
import "./lidarr-request.css";

/** The request preview and confirm, loaded on the first Request. */
const LidarrRequestDialog = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

const n = (v: number) => v.toLocaleString();

/** `songs` false (library import is off): only the albums added by hand. */
export function MissingView({ songs: withSongs = true }: { songs?: boolean }) {
  const [songs, setSongs] = useState<MissingSong[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [focus] = useState(missingFocusKey);
  const ready = useLidarrReady();
  const [asking, setAsking] = useState<{ key: string; input: LidarrRequestInput } | null>(null);
  const [states, setStates] = useState<Record<string, LidarrRequestState>>({});
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    if (withSongs) backend.importMissing().then(setSongs, (e) => setErr(errText(e) || "Couldn't load missing songs"));
  }, [withSongs]);

  // "See in Missing": bring that song into view once the list is there.
  useLayoutEffect(() => {
    if (!songs || !focus) return;
    clearMissingFocus();
    const i = songs.findIndex((s) => missingKey(s.artist, s.title) === focus);
    if (i < 0) return;
    const el = listRef.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(focus)}"]`);
    if (el) {
      el.scrollIntoView?.({ block: "center" });
      el.focus({ preventScroll: true });
      return;
    }
    // A long list renders only the rows on screen: scroll to where it sits.
    const sc = listRef.current?.closest<HTMLElement>(".main__scroll");
    if (sc && listRef.current) sc.scrollTop += listRef.current.getBoundingClientRect().top - sc.getBoundingClientRect().top + i * IMPORT_ROW_H;
  }, [songs, focus]);

  const open = (url: string) => backend.importOpenLink(url).catch((e) => toast.error(`Couldn't open the link${errText(e) ? `: ${errText(e)}` : ""}`));

  /** Find the official album the song comes from, then ask. */
  const request = async (s: MissingSong) => {
    const key = missingKey(s.artist, s.title);
    try {
      const picked = await backend.lidarrPick({ artist: s.artist, title: s.title, song: true });
      if (!picked.group) {
        toast.info(picked.reason ?? "There's no official release of this song for Lidarr to get.");
        return;
      }
      setAsking({ key, input: { releaseGroupId: picked.group.id, title: picked.group.title, artist: s.artist } });
    } catch (e) {
      toast.error(errText(e) || "Couldn't look this song up. Try again.");
    }
  };

  const albums = <MissingAlbums open={open} canRequest={ready} onRequest={(key, input) => setAsking({ key, input })} states={states} alone={!withSongs} />;
  if (!withSongs)
    return (
      <>
        <h1 className="page-title">Missing</h1>
        <p className="import-page__sub">Albums you added from pages for music that isn't in your library.</p>
        {albums}
        {asking && (
          <Suspense fallback={null}>
            <LidarrRequestDialog
              open
              input={asking.input}
              onClose={() => setAsking(null)}
              onDone={(r) => {
                setStates((m) => ({ ...m, [asking.key]: r.state }));
                setAsking(null);
              }}
            />
          </Suspense>
        )}
      </>
    );

  return (
    <>
      <h1 className="page-title">Missing songs</h1>
      <p className="import-page__sub">Songs from your imported playlists and likes that aren't in your library, the most wanted first.</p>
      {albums}
      {err && (
        <p className="import-empty" role="alert">
          {err}
        </p>
      )}
      {!songs && !err && <SkeletonLines lines={6} widths={["90%", "85%", "88%", "80%", "86%", "70%"]} gap={20} />}
      {songs && songs.length === 0 && (
        <p className="import-empty" role="status">
          Every imported song is in your library.
        </p>
      )}
      {songs && songs.length > 0 && (
        <ul className="import-rows" aria-label="Missing songs" ref={listRef}>
          <li className="import-row missing-row import-row--header" aria-hidden="true">
            <span className="import-row__n">#</span>
            <span>Title</span>
            <span>Album</span>
            <span>Find it</span>
          </li>
          <VirtualRows
            count={songs.length}
            rowHeight={IMPORT_ROW_H}
            renderRow={(i) => {
              const s = songs[i];
              const key = missingKey(s.artist, s.title);
              return (
                <MissingRow
                  key={key}
                  s={s}
                  i={i}
                  focused={key === focus}
                  open={open}
                  canRequest={ready}
                  state={states[key] ?? null}
                  onRequest={() => request(s)}
                />
              );
            }}
          />
        </ul>
      )}
      {asking && (
        <Suspense fallback={null}>
          <LidarrRequestDialog
            open
            input={asking.input}
            onClose={() => setAsking(null)}
            onDone={(r) => {
              setStates((m) => ({ ...m, [asking.key]: r.state }));
              setAsking(null);
            }}
          />
        </Suspense>
      )}
    </>
  );
}

/** Albums added to Missing by hand from a page for music that isn't in the library. */
function MissingAlbums({
  open,
  canRequest,
  onRequest,
  states,
  alone,
}: {
  open: (url: string) => unknown;
  canRequest: boolean;
  onRequest: (key: string, input: LidarrRequestInput) => void;
  states: Record<string, LidarrRequestState>;
  /** The page has nothing else (no Songs heading after it). */
  alone: boolean;
}) {
  const [albums, setAlbums] = useState<MissingAlbum[] | null>(null);
  useEffect(() => {
    backend.importMissingAlbums().then(setAlbums, () => setAlbums([]));
  }, []);
  if (!albums) return null;
  if (albums.length === 0)
    return alone ? (
      <p className="import-empty" role="status">
        Nothing on your Missing list. Add albums from their pages to remember them.
      </p>
    ) : null;
  const remove = (a: MissingAlbum) =>
    backend.importMissingAlbumRemove(a.releaseGroupId).then(
      () => setAlbums((l) => (l ?? []).filter((x) => x.releaseGroupId !== a.releaseGroupId)),
      (e) => toast.error(errText(e) || "Couldn't take it off Missing"),
    );
  return (
    <section className="missing-albums" aria-labelledby="missing-albums-title">
      <h2 id="missing-albums-title" className="section-title">
        Albums you added
      </h2>
      <ul className="import-rows" aria-label="Albums you added">
        {albums.map((a) => {
          const key = `album:${a.releaseGroupId}`;
          const state = states[key];
          return (
            <li key={a.releaseGroupId} className="import-row missing-row" aria-label={`${a.title} by ${a.artist}`}>
              <span className="import-row__n" aria-hidden="true" />
              <span className="import-row__song">
                <Art path={a.thumb} size={40} className="import-row__art" />
                <span className="import-row__main">
                  <button
                    type="button"
                    className="link import-row__title truncate"
                    onClick={() => nav.go(a.libraryAlbumId ? { name: "album", id: a.libraryAlbumId } : { name: "catalogueAlbum", id: a.releaseGroupId })}
                  >
                    {a.title}
                  </button>
                  <span className="import-row__artist truncate">{a.artist}</span>
                </span>
              </span>
              <span className="import-row__album truncate">{a.libraryAlbumId ? "In your library now" : "Album"}</span>
              <span className="missing-row__links">
                {!a.libraryAlbumId &&
                  STORE_LINKS.map((st) => (
                    <Button key={st.key} size="sm" variant="ghost" onClick={() => void open(a.links[st.key])} aria-label={`Search ${st.name} for ${a.title}`}>
                      {st.name}
                    </Button>
                  ))}
                {canRequest &&
                  !a.libraryAlbumId &&
                  (state && state !== "failed" ? (
                    <span className={`lidarr-state lidarr-state--${state}`}>
                      {state === "queued" ? <Clock size={14} aria-hidden="true" /> : <Download size={14} aria-hidden="true" />}
                      {STATE_LABEL[state]}
                    </span>
                  ) : (
                    <Button
                      size="sm"
                      aria-label={`Request ${a.title}`}
                      onClick={() => onRequest(key, { releaseGroupId: a.releaseGroupId, title: a.title, artist: a.artist, artistMbid: a.artistMbid ?? null })}
                    >
                      Request
                    </Button>
                  ))}
                <Button size="sm" variant="ghost" onClick={() => void remove(a)} aria-label={`Remove ${a.title} from Missing`}>
                  Remove
                </Button>
              </span>
            </li>
          );
        })}
      </ul>
      {!alone && <h2 className="section-title missing-albums__songs">Songs</h2>}
    </section>
  );
}

function MissingRow({
  s,
  i,
  focused,
  open,
  canRequest,
  state,
  onRequest,
}: {
  s: MissingSong;
  i: number;
  focused: boolean;
  open: (url: string) => unknown;
  canRequest: boolean;
  state: LidarrRequestState | null;
  onRequest: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <li
      className={`import-row missing-row ${focused ? "is-focused" : ""}`}
      aria-label={`${s.title} by ${s.artist}`}
      data-key={missingKey(s.artist, s.title)}
      tabIndex={focused ? -1 : undefined}
    >
      <span className="import-row__n">{i + 1}</span>
      <span className="import-row__song">
        <Art path={null} size={40} className="import-row__art" />
        <span className="import-row__main">
          <span className="import-row__title truncate">{s.title}</span>
          <span className="import-row__artist truncate">{s.artist}</span>
          <span className="missing-row__wants" title={s.wantedBy.join(", ")}>
            Wanted by {s.wants === 1 ? (s.wantedBy[0] ?? "1 list") : `${n(s.wants)} lists`}
          </span>
        </span>
      </span>
      <span className="import-row__album truncate">{s.album ?? ""}</span>
      <span className="missing-row__links">
        {STORE_LINKS.map((st) => (
          <Button key={st.key} size="sm" variant="ghost" onClick={() => void open(s.links[st.key])} aria-label={`Search ${st.name} for ${s.title}`}>
            {st.name}
          </Button>
        ))}
        {canRequest &&
          (state && state !== "failed" ? (
            <span className={`lidarr-state lidarr-state--${state}`}>
              {state === "queued" ? <Clock size={14} aria-hidden="true" /> : <Download size={14} aria-hidden="true" />}
              {STATE_LABEL[state]}
            </span>
          ) : (
            <Button
              size="sm"
              disabled={busy}
              aria-label={`Request ${s.title}`}
              onClick={() => {
                setBusy(true);
                void onRequest().finally(() => setBusy(false));
              }}
            >
              {busy ? "Finding…" : "Request"}
            </Button>
          ))}
      </span>
    </li>
  );
}
