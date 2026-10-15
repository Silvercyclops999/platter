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
import type { LidarrRequestInput, LidarrRequestState } from "../lib/types";
import { toast } from "../stores/toasts";
import { IMPORT_ROW_H } from "./ImportListView";
import { clearMissingFocus, errText, missingFocusKey, missingKey } from "./importShared";
import { STATE_LABEL, useLidarrReady } from "./lidarrShared";
import "./lidarr-request.css";

/** The request preview and confirm, loaded on the first Request. */
const LidarrRequestDialog = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

const n = (v: number) => v.toLocaleString();

export function MissingView() {
  const [songs, setSongs] = useState<MissingSong[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [focus] = useState(missingFocusKey);
  const ready = useLidarrReady();
  const [asking, setAsking] = useState<{ key: string; input: LidarrRequestInput } | null>(null);
  const [states, setStates] = useState<Record<string, LidarrRequestState>>({});
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    backend.importMissing().then(setSongs, (e) => setErr(errText(e) || "Couldn't load missing songs"));
  }, []);

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

  return (
    <>
      <h1 className="page-title">Missing songs</h1>
      <p className="import-page__sub">Songs from your imported playlists and likes that aren't in your library, the most wanted first.</p>
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
