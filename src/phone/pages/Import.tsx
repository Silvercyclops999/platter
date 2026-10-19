/**
 * Library import on the phone: Bring in my listening (its switch and where
 * the waves stand), each source's report, the matches to check, imported
 * playlists and the Missing list, whose rows have where to buy and Get
 * (Lidarr). The same core calls as the computer's import pages.
 */
import { Heart, ListMusic, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Art } from "../../components/Art";
import { Switch } from "../../components/Switch";
import { backend } from "../../lib/backend";
import { failText } from "../../lib/errorText";
import { formatDuration } from "../../lib/format";
import { STORE_LINKS, importSourceName } from "../../lib/importStrings";
import type { ImportReport, ImportReviewItem, MissingSong } from "../../lib/importTypes";
import type { BringInStatus, LidarrRequestInput, LidarrRequestState, MissingAlbum, StoreLinkSet } from "../../lib/types";
import { stateLine, waveLine, whereLine } from "../../pages/BringIn";
import { openStore } from "../../pages/catalogue";
import { RequestDialog } from "../../pages/catalogue";
import { ListView } from "../../pages/ImportListView";
import { countParts, errText, historyParts, importChanged, missingKey, reportLine, reportPercent, songsLine, useImportOverview } from "../../pages/importShared";
import { STATE_LABEL, useLidarrReady } from "../../pages/lidarrShared";
import { PlayedALot } from "../../pages/PlayedALot";
import { nav } from "../../stores/router";
import { updateSettings, useSettings } from "../../stores/settings";
import { toast } from "../../stores/toasts";
import { Borrowed, Empty, GhostRows, GoPill, LinkList, PageFrame, Pills, Row, ShowMore } from "./kit";
import "../../pages/library-import.css";

const n = (v: number) => v.toLocaleString();

// ---------------------------------------------------------------- bring in my listening

/** Bring in my listening: on or off, and while on, the wave, today's count and what needs you. */
export function BringInPhone() {
  const on = useSettings((s) => !!s.settings?.bringIn);
  const [s, setS] = useState<BringInStatus | null>(null);
  const seq = useRef(0);
  const load = useCallback(() => {
    const mine = ++seq.current;
    backend.bringInStatus().then((v) => mine === seq.current && setS(v), () => {});
  }, []);
  useEffect(() => {
    if (on) load();
  }, [on, load]);
  const act = (p: Promise<BringInStatus>) => {
    const mine = ++seq.current;
    return p.then((v) => mine === seq.current && setS(v), (e) => toast.error(errText(e) || "Couldn't change Bring in my listening"));
  };
  return (
    <section className="ph-card ph-bring" aria-label="Bring in my listening">
      <div className="ph-bring__head">
        <span className="ph-row__text">
          <span className="ph-card__title" id="ph-bring-title">
            Bring in my listening
          </span>
          <span className="ph-row__sub ph-bring__sub">Asks Lidarr for your most played albums, a few a day.</span>
        </span>
        <Switch checked={on} onChange={(v) => void updateSettings({ bringIn: v })} labelledBy="ph-bring-title" />
      </div>
      {on && s && (
        <div className="ph-bring__body">
          <p className="ph-bring__wave">{waveLine(s)}</p>
          <p className="ph-x__meta tabular">
            {n(s.done)} done · {n(s.remaining)} left in this wave · {n(s.today)} of {n(s.perDay)} today
          </p>
          <p className="ph-x__meta" role="status">
            {stateLine(s)}
          </p>
          {whereLine(s) && <p className="ph-x__meta">{whereLine(s)}</p>}
          <div className="ph-x__pill-row">
            <button type="button" className="ph-pill" onClick={() => void act(backend.bringInPause(!s.userPaused))}>
              {s.userPaused ? "Carry on" : "Pause"}
            </button>
            <GoPill route={{ name: "settings", page: "libraryImport", row: "bringIn" }}>Waves and limits</GoPill>
          </div>
          {s.needsYou.length > 0 && (
            <>
              <h3 className="ph-label ph-bring__needs">Needs you ({n(s.needsYou.length)})</h3>
              <ul className="ph-rows">
                {s.needsYou.map((x) => (
                  <li key={x.key} className="ph-bring__need">
                    <p className="ph-row__title">{x.album}</p>
                    <p className="ph-row__sub">
                      {x.artist} · {n(x.plays)} plays
                    </p>
                    <p className="ph-x__meta">{x.reason}</p>
                    <div className="ph-x__pill-row">
                      <button type="button" className="ph-pill" aria-label={`Try ${x.album} again`} onClick={() => void act(backend.bringInNeedsYou(x.key, true))}>
                        Try again
                      </button>
                      <button type="button" className="ph-pill" aria-label={`Leave ${x.album}`} onClick={() => void act(backend.bringInNeedsYou(x.key, false))}>
                        Leave it
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </section>
  );
}

// ---------------------------------------------------------------- reports and checks

function ReportCard({ r }: { r: ImportReport }) {
  const pct = reportPercent(r);
  const history = historyParts(r);
  const head = r.songs.total ? songsLine(r) : history ? history.head : reportLine(r);
  const counts = [
    ...(r.songs.unsure > 0 ? [`${n(r.songs.unsure)} to check`] : []),
    ...(r.songs.missing > 0 ? [`${n(r.songs.missing)} not in your library`] : []),
    ...countParts(r),
    ...(history ? [history.songs] : []),
  ];
  return (
    <article className="ph-card ph-report" aria-label={`${importSourceName(r.source)} import`} data-testid={`report-${r.source}`}>
      <p className="ph-label">{importSourceName(r.source)}</p>
      <p className="ph-report__head">{head}</p>
      <div className="ph-report__bar" role="img" aria-label={`${pct}% found`}>
        <span style={{ width: `${pct}%` }} />
      </div>
      {counts.length > 0 && <p className="ph-x__meta">{counts.join(" · ")}</p>}
    </article>
  );
}

function Review({ item, onDone }: { item: ImportReviewItem; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const decide = (d: Parameters<typeof backend.importReview>[1]) => {
    setBusy(true);
    backend.importReview(item.songId, d).then(
      () => {
        importChanged();
        onDone();
      },
      (e) => {
        setBusy(false);
        toast.error(failText(`Couldn't save your choice for ${item.title}`, e));
      },
    );
  };
  return (
    <li className="ph-review" aria-label={`${item.title} by ${item.artist}`}>
      <p className="ph-row__title">{item.title}</p>
      <p className="ph-row__sub">{[item.artist, item.album, item.durationMs ? formatDuration(item.durationMs) : null, `from ${importSourceName(item.source)}`].filter(Boolean).join(" · ")}</p>
      <ul className="ph-rows">
        {item.candidates.map((t, i) => (
          <Row
            key={t.id}
            title={
              <>
                {t.title} {i === 0 && <span className="ph-x__tag is-accent">Best match</span>}
              </>
            }
            sub={`${t.trackArtist ?? t.artistTitle} · ${t.albumTitle} · ${formatDuration(t.durationMs)}`}
            end={
              <button
                type="button"
                className={`ph-pill ${i === 0 ? "is-primary" : ""}`}
                disabled={busy}
                aria-label={i === 0 ? `Accept ${t.title} on ${t.albumTitle}` : `Use ${t.title} on ${t.albumTitle}`}
                onClick={() => decide(i === 0 ? { kind: "accept" } : { kind: "pick", trackId: t.id })}
              >
                {i === 0 ? "Accept" : "Use"}
              </button>
            }
          />
        ))}
      </ul>
      <button type="button" className="ph-text-btn" disabled={busy} onClick={() => decide({ kind: "none" })}>
        None of these
      </button>
    </li>
  );
}

function ReviewQueue() {
  const [items, setItems] = useState<ImportReviewItem[] | null>(null);
  const load = () => backend.importReviewQueue().then(setItems, () => setItems([]));
  useEffect(() => {
    void load();
  }, []);
  if (!items?.length) return null;
  return (
    <section className="ph-x__section" aria-label="Check these matches">
      <h2 className="ph-section-title">Check these matches</h2>
      <p className="ph-section-sub">These might be songs you have, but Platter isn't sure. Your answer is kept when you import again.</p>
      <ul className="ph-rows">
        {items.map((it) => (
          <Review key={it.songId} item={it} onDone={() => void load()} />
        ))}
      </ul>
    </section>
  );
}

function ImportHome() {
  const [o, err] = useImportOverview();
  if (err) return <Empty title="Couldn't load the import" text={err} />;
  if (!o) return <GhostRows />;
  return (
    <>
      {o.reports.length === 0 ? (
        <Empty title="Nothing imported yet" text="Pick a file for a service, or connect an account, in Settings under Library import." action={<GoPill route={{ name: "settings", page: "libraryImport" }}>Open Settings</GoPill>} />
      ) : (
        <div className="ph-reports">
          {o.reports.map((r) => (
            <ReportCard key={r.source} r={r} />
          ))}
        </div>
      )}
      <ReviewQueue key={o.reviewCount} />
      <LinkList
        label="Imported"
        items={[
          ...(o.missingCount > 0 ? [{ title: "Missing songs", sub: `${n(o.missingCount)} from your imports aren't in your library`, route: { name: "import", view: "missing" } as const }] : []),
          ...o.lists.map((l) => ({
            title: l.name,
            sub: `${importSourceName(l.source)} · ${n(l.matched)} of ${n(l.total)} in your library`,
            icon: l.liked ? <Heart size={18} /> : <ListMusic size={18} />,
            route: { name: "import", view: `list:${l.id}` } as const,
          })),
        ]}
      />
    </>
  );
}

// ---------------------------------------------------------------- missing

function RequestState({ state }: { state: LidarrRequestState }) {
  return <span className={`lidarr-state lidarr-state--${state}`}>{STATE_LABEL[state]}</span>;
}

function StorePills({ links, title }: { links: StoreLinkSet; title: string }) {
  return (
    <>
      {STORE_LINKS.map((st) => (
        <button key={st.key} type="button" className="ph-pill" aria-label={`Search ${st.name} for ${title}`} onClick={() => void openStore(links[st.key])}>
          {st.name}
        </button>
      ))}
    </>
  );
}

function MissingSongRow({ s, ready, state, onRequest }: { s: MissingSong; ready: boolean; state: LidarrRequestState | null; onRequest: () => Promise<void> }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  return (
    <li className="ph-missing" aria-label={`${s.title} by ${s.artist}`}>
      <button type="button" className="ph-row" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className="ph-row__text">
          <span className="ph-row__title">{s.title}</span>
          <span className="ph-row__sub">{[s.artist, s.album, `wanted by ${s.wants === 1 ? (s.wantedBy[0] ?? "1 list") : `${n(s.wants)} lists`}`].filter(Boolean).join(" · ")}</span>
        </span>
        {state && state !== "failed" && <RequestState state={state} />}
      </button>
      {open && (
        <div className="ph-x__pill-row ph-get">
          {ready && (!state || state === "failed") && (
            <button
              type="button"
              className="ph-pill is-primary"
              disabled={busy}
              aria-label={`Request ${s.title}`}
              onClick={() => {
                setBusy(true);
                void onRequest().finally(() => setBusy(false));
              }}
            >
              {busy ? "Finding…" : "Get"}
            </button>
          )}
          <StorePills links={s.links} title={s.title} />
        </div>
      )}
    </li>
  );
}

function MissingAlbums({ ready, states, onRequest }: { ready: boolean; states: Record<string, LidarrRequestState>; onRequest: (key: string, input: LidarrRequestInput) => void }) {
  const [albums, setAlbums] = useState<MissingAlbum[] | null>(null);
  useEffect(() => {
    backend.importMissingAlbums().then(setAlbums, () => setAlbums([]));
  }, []);
  if (!albums?.length) return null;
  const remove = (a: MissingAlbum) =>
    backend.importMissingAlbumRemove(a.releaseGroupId).then(
      () => setAlbums((l) => (l ?? []).filter((x) => x.releaseGroupId !== a.releaseGroupId)),
      (e) => toast.error(errText(e) || "Couldn't take it off Missing"),
    );
  return (
    <section className="ph-x__section" aria-label="Albums you added">
      <h2 className="ph-section-title">Albums you added</h2>
      <ul className="ph-rows">
        {albums.map((a) => {
          const key = `album:${a.releaseGroupId}`;
          const state = states[key];
          return (
            <li key={a.releaseGroupId} className="ph-missing" aria-label={`${a.title} by ${a.artist}`}>
              <div className="ph-x__row">
                <button type="button" className="ph-row" onClick={() => nav.go(a.libraryAlbumId ? { name: "album", id: a.libraryAlbumId } : { name: "catalogueAlbum", id: a.releaseGroupId })}>
                  <Art path={a.thumb} size={56} kind="album" sleeve={{ title: a.title, artist: a.artist }} />
                  <span className="ph-row__text">
                    <span className="ph-row__title">{a.title}</span>
                    <span className="ph-row__sub">{a.libraryAlbumId ? `${a.artist} · In your library now` : a.artist}</span>
                  </span>
                </button>
                <button type="button" className="ph-icon-btn" aria-label={`Remove ${a.title} from Missing`} onClick={() => void remove(a)}>
                  <X size={20} aria-hidden="true" />
                </button>
              </div>
              {!a.libraryAlbumId && (
                <div className="ph-x__pill-row ph-get">
                  {ready &&
                    (state && state !== "failed" ? (
                      <RequestState state={state} />
                    ) : (
                      <button
                        type="button"
                        className="ph-pill is-primary"
                        aria-label={`Request ${a.title}`}
                        onClick={() => onRequest(key, { releaseGroupId: a.releaseGroupId, title: a.title, artist: a.artist, artistMbid: a.artistMbid ?? null })}
                      >
                        Get
                      </button>
                    ))}
                  <StorePills links={a.links} title={a.title} />
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function MissingPhone({ played }: { played: boolean }) {
  const s = useSettings((x) => x.settings);
  const withSongs = (s?.importSources ?? []).length > 0;
  const showPlayed = s?.showPlayedALot !== false;
  const ready = useLidarrReady();
  const [songs, setSongs] = useState<MissingSong[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [shown, setShown] = useState(60);
  const [states, setStates] = useState<Record<string, LidarrRequestState>>({});
  const [asking, setAsking] = useState<{ key: string; input: LidarrRequestInput } | null>(null);
  useEffect(() => {
    if (withSongs && !played) backend.importMissing().then(setSongs, (e) => setErr(errText(e) || "Couldn't load missing songs"));
  }, [withSongs, played]);
  const request = async (m: MissingSong) => {
    try {
      const picked = await backend.lidarrPick({ artist: m.artist, title: m.title, song: true });
      if (!picked.group) return void toast.info(picked.reason ?? "There's no official release of this song for Lidarr to get");
      setAsking({ key: missingKey(m.artist, m.title), input: { releaseGroupId: picked.group.id, title: picked.group.title, artist: m.artist } });
    } catch (e) {
      toast.error(errText(e) || "Couldn't look this song up. Try again");
    }
  };
  return (
    <PageFrame name="import" title="Missing" lede={played ? "Listening history with no match in your library." : "What you want that isn't in your library, the most wanted first."}>
      {showPlayed && (
        <Pills
          label="What's missing"
          items={[
            { id: "missing", label: withSongs ? "From your imports" : "Albums you added" },
            { id: "played", label: "Played a lot" },
          ]}
          value={played ? "played" : "missing"}
          onChange={(v) => nav.go({ name: "import", view: v })}
        />
      )}
      <BringInPhone />
      {played ? (
        <div className="ph-wrap-in">
          <PlayedALot />
        </div>
      ) : (
        <>
          <MissingAlbums ready={ready} states={states} onRequest={(key, input) => setAsking({ key, input })} />
          {withSongs && (
            <section className="ph-x__section" aria-label="Missing songs">
              <h2 className="ph-section-title">Songs</h2>
              {err ? (
                <p className="ph-x__note">{err}</p>
              ) : !songs ? (
                <GhostRows />
              ) : songs.length === 0 ? (
                <Empty title="Nothing missing" text="Every imported song is in your library." />
              ) : (
                <>
                  <ul className="ph-rows" aria-label="Missing songs">
                    {songs.slice(0, shown).map((m) => {
                      const key = missingKey(m.artist, m.title);
                      return <MissingSongRow key={key} s={m} ready={ready} state={states[key] ?? null} onRequest={() => request(m)} />;
                    })}
                  </ul>
                  <ShowMore total={songs.length} shown={shown} noun="songs" onMore={() => setShown((x) => x + 60)} />
                </>
              )}
            </section>
          )}
        </>
      )}
      <RequestDialog
        input={asking?.input ?? null}
        onClose={() => setAsking(null)}
        onDone={() => {
          if (asking) setStates((m) => ({ ...m, [asking.key]: "requested" }));
          setAsking(null);
        }}
      />
    </PageFrame>
  );
}

// ---------------------------------------------------------------- page

export function PhoneImport({ view }: { view?: string }) {
  const s = useSettings((x) => x.settings);
  const on = (s?.importSources ?? []).length > 0;
  if (view === "missing" || view === "played") return <MissingPhone played={view === "played"} />;
  if (view?.startsWith("list:"))
    return (
      <Borrowed name="import">
        <ListView id={view.slice(5)} />
      </Borrowed>
    );
  return (
    <PageFrame name="import" title="Library import" lede="How much of what you imported is in your library.">
      <BringInPhone />
      {s && !on ? (
        <Empty title="Library import is off" text="Turn on a service in Settings, under Library import." action={<GoPill route={{ name: "settings", page: "libraryImport" }}>Open Settings</GoPill>} />
      ) : (
        <ImportHome />
      )}
      <LinkList label="More" items={[{ title: "Missing", sub: "Albums you added and songs you want", route: { name: "import", view: "missing" } }]} />
    </PageFrame>
  );
}
