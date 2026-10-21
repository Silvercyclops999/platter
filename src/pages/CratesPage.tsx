/**
 * Crates: the library filed the way a record shop would, by label,
 * producer, studio, era and genre. Each crate is a row of sleeves you flip
 * through (arrow keys move from record to record); a crate's page holds all
 * of it, with Play all and Shuffle. The core groups the albums (`crates`,
 * `crate_page`) from tags, Plex and cached sleeve notes.
 */
import { AudioLines, ChevronLeft, ChevronRight, Disc3, Play, Shuffle } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Button, IconButton } from "../components/Button";
import { LoadError } from "../components/LoadError";
import { SearchField } from "../components/SearchField";
import { Skeleton } from "../components/Skeleton";
import { Tabs } from "../components/Tabs";
import { prefersReducedMotion } from "../design/motion";
import { backend } from "../lib/backend";
import type { Album, Crate, CrateKind } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { player } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useSoundMapOn } from "../stores/soundmap";
import { AlbumCard } from "./cards";
import { CRATE_NOUN, CRATE_PLATE, CRATE_SOURCE, CRATE_TAB_LABEL, CRATE_TABS, crateLine, crateSentence, records } from "./crates";
import { SleeveCard, useStudioLook } from "./RecordHome";
import { openSetting } from "./SoundMapPage";
import "./record-home.css";
import "./crates.css";

/** Crates drawn at first; more on request. */
const FIRST_CRATES = 12;
/** Sleeves drawn at first on a crate's page. */
const FIRST_SLEEVES = 60;

type Sort = "size" | "name";

const useCratesOn = () => useSettings((s) => s.settings?.showCrates !== false);

/** Focusable record links in a row, in order. */
const LINKS = ".sleeve-card__link, .card__link, .crate__all";

/**
 * Left and Right flip to the next record in the row, Home and End to the
 * first and last; the row slides along with a calm scroll (none under
 * reduced motion).
 */
export function flipThrough(e: KeyboardEvent<HTMLElement>) {
  const links = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(LINKS));
  const at = links.findIndex((l) => l === document.activeElement || l.closest("[role=listitem]")?.contains(e.target as Node));
  if (at < 0) return;
  const next = e.key === "ArrowRight" ? at + 1 : e.key === "ArrowLeft" ? at - 1 : e.key === "Home" ? 0 : e.key === "End" ? links.length - 1 : -1;
  if (next < 0 || next >= links.length) return;
  e.preventDefault();
  if (next === at) return;
  links[next].focus({ preventScroll: true });
  const item = links[next].closest<HTMLElement>("[role=listitem]") ?? links[next];
  item.scrollIntoView?.({ block: "nearest", inline: "nearest", behavior: prefersReducedMotion() ? "auto" : "smooth" });
}

/** A record in a crate: a sleeve in Platter's own looks, a card in Classic. */
function Record({ album, meta, studio, extra }: { album: Album; meta: string; studio: boolean; extra?: React.ReactNode }) {
  if (studio) return <SleeveCard album={album} meta={meta} extra={extra} />;
  return (
    <div role="listitem" className="crate__card">
      <AlbumCard album={album} />
      {extra}
    </div>
  );
}

const metaFor = (a: Album, kind: CrateKind) => (kind === "year" ? (a.genres[0] ?? a.artistTitle) : a.year ? String(a.year) : "No year");

/** One crate as a row of sleeves with a divider card naming it. */
export function CrateRow({ crate }: { crate: Crate }) {
  const id = useId();
  const studio = useStudioLook();
  const row = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ start: true, end: false });
  const open = () => nav.go({ name: "crate", kind: crate.kind, key: crate.key });
  const measure = () => {
    const el = row.current;
    if (!el) return;
    const start = el.scrollLeft <= 2;
    const end = el.scrollLeft + el.clientWidth >= el.scrollWidth - 2;
    setEdges((e) => (e.start === start && e.end === end ? e : { start, end }));
  };
  useEffect(measure, [crate.albums.length]);
  const flip = (dir: 1 | -1) => {
    const el = row.current;
    if (!el) return;
    el.scrollBy({ left: dir * el.clientWidth * 0.75, behavior: prefersReducedMotion() ? "auto" : "smooth" });
  };
  const more = crate.count > crate.albums.length;
  return (
    <section className="crate" aria-labelledby={id} data-testid="crate">
      <header className="crate__divider">
        <div className="crate__titles">
          <h2 className="crate__name" id={id}>
            <button type="button" className="crate__open" onClick={open}>
              {crate.name}
            </button>
          </h2>
          <p className="crate__line">{crateLine(crate)}</p>
        </div>
        <div className="crate__tools">
          {!edges.start && <IconButton size="sm" tooltip={false} label={`Flip back through ${crate.name}`} icon={<ChevronLeft size={18} />} onClick={() => flip(-1)} />}
          {!edges.end && crate.albums.length > 1 && (
            <IconButton size="sm" tooltip={false} label={`Flip on through ${crate.name}`} icon={<ChevronRight size={18} />} onClick={() => flip(1)} />
          )}
        </div>
      </header>
      <div className={`crate__sleeves ${studio ? "is-sleeves" : "is-cards"}`} role="list" aria-label={`${crate.name}: ${records(crate.count)}`} ref={row} onScroll={measure} onKeyDown={flipThrough}>
        {crate.albums.map((a) => (
          <Record key={a.id} album={a} meta={metaFor(a, crate.kind)} studio={studio} />
        ))}
        {more && (
          <div role="listitem" className="crate__rest">
            <button type="button" className="crate__all" onClick={open}>
              <span className="crate__all-count">{crate.count - crate.albums.length}</span>
              <span>more in this crate</span>
            </button>
          </div>
        )}
      </div>
    </section>
  );
}

function CratesOff() {
  return (
    <div className="empty-state" role="status">
      <h2 className="empty-state__title">Crates are turned off</h2>
      <p className="empty-state__text">Turn them on to browse your records by label, producer, studio, era and genre.</p>
      <Button variant="secondary" onClick={() => openSetting("showCrates")}>
        Crates settings
      </Button>
    </div>
  );
}

export function CratesPage({ kind = "label" }: { kind?: CrateKind }) {
  const on = useCratesOn();
  const [sort, setSort] = useState<Sort>("size");
  const [filter, setFilter] = useState("");
  const [shown, setShown] = useState(FIRST_CRATES);
  const { data, error, retry } = useLoad(() => (on ? backend.crates(kind) : Promise.resolve(null)), [kind, on]);
  useEffect(() => setShown(FIRST_CRATES), [kind, filter, sort]);
  const time = kind === "era" || kind === "year";
  const crates = useMemo(() => {
    const list = data?.crates ?? [];
    const q = filter.trim().toLowerCase();
    const hits = q ? list.filter((c) => c.name.toLowerCase().includes(q)) : list;
    if (sort === "name") return [...hits].sort((a, b) => (time ? a.key.localeCompare(b.key) : a.name.localeCompare(b.name, undefined, { sensitivity: "base" })));
    return hits;
  }, [data, filter, sort, time]);
  const [one, many] = CRATE_NOUN[kind];
  const choose = (k: CrateKind) => useRouter.getState().replace({ name: "crates", kind: k });

  return (
    <div className="page-pad crates-page" data-testid="crates-page">
      <header className="crates-page__head">
        <h1 className="page-title crates-page__title">Crates</h1>
        <p className="crates-page__lede">Your records filed the way a shop would file them. Pick a crate and flip through it.</p>
      </header>
      {!on ? (
        <CratesOff />
      ) : (
        <>
          <Tabs<CrateKind>
            label="Crates by"
            idPrefix="crates"
            className="crates-page__tabs"
            value={kind}
            onChange={choose}
            items={CRATE_TABS.map((k) => ({ id: k, label: CRATE_TAB_LABEL[k] }))}
          />
          <div role="tabpanel" id="crates-panel" aria-labelledby={`crates-tab-${kind}`} className="crates-page__panel">
            {error && data === undefined ? (
              <LoadError what="crates" error={error} onRetry={retry} inline />
            ) : data === undefined ? (
              <div aria-busy="true" className="crates-page__loading">
                <Skeleton width={260} height={14} />
                {Array.from({ length: 3 }, (_, i) => (
                  <Skeleton key={i} width="100%" height={180} style={{ margin: "24px 0" }} />
                ))}
              </div>
            ) : data && data.crates.length === 0 ? (
              <div className="empty-state" role="status">
                <h2 className="empty-state__title">No {many} with two records yet</h2>
                <p className="empty-state__text">{CRATE_SOURCE[kind]} A crate needs two records or more.</p>
              </div>
            ) : (
              <>
                <div className="crates-page__bar">
                  <p className="crates-page__count">
                    {data!.crates.length === 1 ? `1 ${one}` : `${data!.crates.length.toLocaleString()} ${many}`} with two records or more.{" "}
                    <span className="crates-page__source">{CRATE_SOURCE[kind]}</span>
                  </p>
                  <div className="crates-page__tools">
                    <SearchField className="crates-page__filter" value={filter} onChange={setFilter} label={`Filter ${many}`} placeholder={`Filter ${many}`} />
                    <div className="crates-page__seg" role="group" aria-label="Sort crates">
                      <button type="button" aria-pressed={sort === "size"} onClick={() => setSort("size")}>
                        {time ? "Newest" : "Most records"}
                      </button>
                      <button type="button" aria-pressed={sort === "name"} onClick={() => setSort("name")}>
                        {time ? "Oldest" : "A to Z"}
                      </button>
                    </div>
                  </div>
                </div>
                {crates.length === 0 ? (
                  <p className="crates-page__none" role="status">
                    No {many} match "{filter.trim()}".
                  </p>
                ) : (
                  crates.slice(0, shown).map((c) => <CrateRow key={c.key} crate={c} />)
                )}
                {crates.length > shown && (
                  <div className="crates-page__more">
                    <Button variant="secondary" onClick={() => setShown((n) => n + FIRST_CRATES)}>
                      Show more {many}
                    </Button>
                  </div>
                )}
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function shuffled<T>(list: T[]): T[] {
  const out = [...list];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

/** One crate: a plate with its name, then every record in it as a sleeve. */
export function CratePage({ kind, crateKey }: { kind: CrateKind; crateKey: string }) {
  const on = useCratesOn();
  const studio = useStudioLook();
  const mapOn = useSoundMapOn();
  const [shown, setShown] = useState(FIRST_SLEEVES);
  const { data, error, retry } = useLoad(() => (on ? backend.cratePage(kind, crateKey) : Promise.resolve(null)), [kind, crateKey, on]);
  if (!on) {
    return (
      <div className="page-pad crate-page">
        <CratesOff />
      </div>
    );
  }
  if (error && data === undefined) return <LoadError what="this crate" error={error} onRetry={retry} />;
  const albums = data?.albums ?? [];
  return (
    <div className="page-pad crate-page" data-testid="crate-page">
      <header className="crate-plate">
        <span className="crate-plate__kind">{CRATE_PLATE[kind]}</span>
        <h1 className="crate-plate__name">{data?.name ?? (data === null ? "Crate" : "")}</h1>
        {data === undefined ? (
          <Skeleton width={320} height={14} />
        ) : data === null ? (
          <p className="crate-plate__line">This crate is empty now. Its records may have left the library or moved to another crate.</p>
        ) : (
          <>
            <p className="crate-plate__line">{crateSentence({ ...data, count: data.albums.length })}</p>
            <div className="crate-plate__actions">
              <Button variant="primary" icon={<Play size={16} fill="currentColor" />} disabled={!data.trackIds.length} onClick={() => void player.playTracks(data.trackIds, data.trackIds[0])}>
                Play all
              </Button>
              <Button
                variant="secondary"
                icon={<Shuffle size={16} />}
                disabled={!data.trackIds.length}
                onClick={() => {
                  const order = shuffled(data.trackIds);
                  void player.playTracks(order, order[0]);
                }}
              >
                Shuffle
              </Button>
              {albums.length > 1 && (
                <Button variant="ghost" icon={<Disc3 size={16} />} onClick={() => void player.shuffleAlbums(albums.map((a) => a.album.id))}>
                  Shuffle albums
                </Button>
              )}
              {kind === "producer" && (
                <Button variant="ghost" onClick={() => nav.go({ name: "credits", person: data.name })}>
                  Everything {data.name} is credited on
                </Button>
              )}
            </div>
            {data.years.length > 0 && (
              <nav className="crate-plate__years" aria-label="Years in this era">
                {data.years.map((y) => (
                  <button key={y} type="button" className="crate-year" onClick={() => nav.go({ name: "crate", kind: "year", key: String(y) })}>
                    {y}
                  </button>
                ))}
              </nav>
            )}
          </>
        )}
      </header>
      {data === undefined ? (
        <div aria-busy="true" className="crate-page__grid">
          {Array.from({ length: 8 }, (_, i) => (
            <Skeleton key={i} width="100%" height={200} />
          ))}
        </div>
      ) : (
        albums.length > 0 && (
          <div className={`crate-page__grid ${studio ? "is-sleeves" : "is-cards"}`} role="list" aria-label={`${data?.name}: records`} onKeyDown={flipThrough}>
            {albums.slice(0, shown).map(({ album, onMap }) => (
              <Record
                key={album.id}
                album={album}
                studio={studio}
                meta={kind === "year" ? (album.genres[0] ?? "") : `${album.year ?? "No year"}`}
                extra={
                  onMap && mapOn ? (
                    <button type="button" className="crate-sounds" onClick={() => nav.go({ name: "soundMap", album: album.id })}>
                      <AudioLines size={14} aria-hidden="true" />
                      <span>Sounds like this record</span>
                    </button>
                  ) : null
                }
              />
            ))}
          </div>
        )
      )}
      {albums.length > shown && (
        <div className="crates-page__more">
          <Button variant="secondary" onClick={() => setShown((n) => n + FIRST_SLEEVES)}>
            Show {Math.min(FIRST_SLEEVES, albums.length - shown)} more
          </Button>
        </div>
      )}
    </div>
  );
}
