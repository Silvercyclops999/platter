/**
 * One imported playlist (or liked list). Songs in the library play like any
 * list; the rest stay as dimmed rows: readable, selectable, with a short
 * menu, and a toast instead of playing. Play, Shuffle and Add to queue skip
 * them. A list whose import was removed opens a "not available" page.
 */
import { Ellipsis, Play, Shuffle } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { Dialog } from "../components/Dialog";
import { addToPlaylistItems } from "../components/PlaylistPicker";
import { Skeleton, SkeletonLines } from "../components/Skeleton";
import { Tooltip } from "../components/Tooltip";
import { trackMenuItems } from "../components/TrackRow";
import { VirtualRows } from "../components/VirtualRows";
import { backend } from "../lib/backend";
import { formatDuration, formatLongDuration } from "../lib/format";
import { STORE_LINKS, importSourceName, storeLinks } from "../lib/importStrings";
import type { ImportedList, ImportedListPage, ImportedRow } from "../lib/importTypes";
import type { AppError, Track } from "../lib/types";
import { setLiked } from "../stores/likes";
import { player } from "../stores/player";
import { createPlaylist } from "../stores/playlists";
import { nav, useRouter } from "../stores/router";
import { toast, useToasts } from "../stores/toasts";
import type { MenuItem } from "../stores/ui";
import { errText, importChanged, missingKey, showInMissing } from "./importShared";
import { failText } from "../lib/errorText";

/** About how tall an import row is (56 px minimum plus the reason line). */
export const IMPORT_ROW_H = 60;

const n = (v: number) => v.toLocaleString();

function artistOf(t: Track): string {
  return t.trackArtist ?? t.artistTitle;
}

/** Why a dimmed row is dimmed, in the tooltip. */
export function dimmedWhy(row: ImportedRow): string {
  return row.status === "unsure"
    ? "Platter found songs like this in your library but isn't sure which one it is. Check it on Library import to play it here."
    : "Platter couldn't find this song in your library, so it can't play. It plays here once it's added.";
}

/** The header line: "10 of 12 songs in your library, 38 min" (the length counts songs that can play). */
export function listHeaderLine(rows: ImportedRow[]): string {
  const playable = rows.filter((r) => r.track);
  const ms = playable.reduce((sum, r) => sum + (r.track?.durationMs ?? 0), 0);
  const count =
    playable.length === rows.length
      ? `${n(rows.length)} ${rows.length === 1 ? "song" : "songs"}`
      : `${n(playable.length)} of ${n(rows.length)} ${rows.length === 1 ? "song" : "songs"} in your library`;
  return playable.length ? `${count}, ${formatLongDuration(ms)}` : count;
}

/** Fisher-Yates on a copy. */
function shuffled<T>(xs: T[]): T[] {
  const a = [...xs];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

const NOTHING_PLAYS = "None of these songs are in your library yet.";

/** A small confirm with counts (the two writes to the library). */
function Confirm({ title, body, confirm, onConfirm, onClose }: { title: string; body: string; confirm: string; onConfirm: () => void; onClose: () => void }) {
  return (
    <Dialog open onClose={onClose} label={title} className="confirm">
      <h2 className="confirm__title">{title}</h2>
      <p className="confirm__body">{body}</p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            onClose();
            onConfirm();
          }}
        >
          {confirm}
        </Button>
      </div>
    </Dialog>
  );
}

// ------------------------------------------------------------------ rows

interface RowProps {
  row: ImportedRow;
  index: number;
  selected: boolean;
  onSelect: () => void;
  onActivate: () => void;
  menu: () => MenuItem[];
}

function Row({ row, index, selected, onSelect, onActivate, menu }: RowProps) {
  const t = row.track;
  const title = t?.title ?? row.title;
  const artist = t ? artistOf(t) : row.artist;
  const album = t?.albumTitle ?? row.album ?? "";
  const ms = t ? t.durationMs : row.durationMs;
  const why = row.status === "unsure" ? "Not sure it's in your library" : "Not in your library";
  const open = (at: { clientX: number; clientY: number } | HTMLElement) => openContextMenu(`${title} options`, menu(), at);
  const key = (e: KeyboardEvent<HTMLLIElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter") {
      e.preventDefault();
      onActivate();
    } else if (e.key === " ") {
      e.preventDefault();
      onSelect();
    } else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
      e.preventDefault();
      open(e.currentTarget);
    }
  };
  const li = (
    <li
      className={`import-row ${t ? "import-row--playable" : "import-row--missing"} ${selected ? "is-selected" : ""}`}
      tabIndex={0}
      onClick={onSelect}
      onDoubleClick={onActivate}
      onKeyDown={key}
      onContextMenu={(e: MouseEvent) => {
        e.preventDefault();
        onSelect();
        open(e);
      }}
      aria-label={t ? `${title} by ${artist}` : `${title} by ${artist}, ${why.toLowerCase()}`}
      data-status={t ? "matched" : row.status}
      data-selected={selected || undefined}
    >
      <span className="import-row__n">{index + 1}</span>
      <span className="import-row__song">
        <Art path={t?.thumb ?? null} size={40} className="import-row__art" />
        <span className="import-row__main">
          <span className="import-row__title truncate">{title}</span>
          <span className="import-row__artist truncate">{artist}</span>
          {!t && <span className="import-row__why">{why}</span>}
        </span>
      </span>
      <span className="import-row__album truncate">{album}</span>
      <span className="import-row__dur">{ms ? formatDuration(ms) : ""}</span>
      <span className="import-row__more">
        <IconButton
          label={`More options for ${title}`}
          tooltip={false}
          size="sm"
          tabIndex={-1}
          icon={<Ellipsis size={18} />}
          onClick={(e) => {
            e.stopPropagation();
            onSelect();
            open(e.currentTarget);
          }}
          onDoubleClick={(e) => e.stopPropagation()}
        />
      </span>
    </li>
  );
  if (t) return li;
  return (
    <Tooltip label={dimmedWhy(row)} wrap describe delayMs={500}>
      {li}
    </Tooltip>
  );
}

// ------------------------------------------------------------------ gone

/** A list that's gone entirely (its import was removed). */
export function UnavailableList({ id }: { id: string }) {
  const remove = () => {
    // Back and Forward stop landing here; the list itself is already gone.
    useRouter.getState().drop({ name: "import", view: `list:${id}` });
    nav.go({ name: "import" });
  };
  return (
    <section className="import-gone" aria-labelledby="import-gone-h">
      <h1 id="import-gone-h" className="import-gone__title">
        This playlist isn't available
      </h1>
      <p className="import-gone__why">Its import was removed from Platter, so its songs aren't kept any more.</p>
      <div className="import-gone__actions">
        <Button variant="primary" onClick={remove}>
          Remove from your library
        </Button>
        <Button onClick={() => nav.go({ name: "home" })}>Home</Button>
      </div>
    </section>
  );
}

// ------------------------------------------------------------------ list

export function ListView({ id }: { id: string }) {
  const [page, setPage] = useState<ImportedListPage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [gone, setGone] = useState(false);
  const [ask, setAsk] = useState(false);
  const [selected, setSelected] = useState<number | null>(null);
  // The Missing list, read once when a dimmed row is first opened.
  const missing = useRef<Promise<Set<string>> | null>(null);

  const load = useCallback(() => {
    backend.importListPage(id).then(
      (p) => {
        setPage(p);
        setErr(null);
      },
      (e) => {
        if ((e as AppError | undefined)?.kind === "notFound") setGone(true);
        else setErr(errText(e) || "Couldn't load this list");
      },
    );
  }, [id]);
  useEffect(load, [load]);

  if (gone) return <UnavailableList id={id} />;
  if (err) return <p className="import-empty" role="alert">{err}</p>;
  if (!page) {
    return (
      <div aria-busy="true">
        <Skeleton width={280} height={40} />
        <div style={{ height: 24 }} />
        <SkeletonLines lines={6} widths={["90%", "85%", "88%", "80%", "86%", "70%"]} gap={20} />
      </div>
    );
  }

  const { list, rows } = page;
  const tracks = rows.map((r) => r.track).filter((t): t is Track => !!t);
  const ids = [...new Set(tracks.map((t) => t.id))];
  const left = rows.length - tracks.length;
  const leftNote = left ? ` ${n(left)} ${left === 1 ? "song isn't" : "songs aren't"} in your library and stay out.` : "";
  const nothing = () => toast.info(NOTHING_PLAYS);
  const play = () => (ids.length ? void player.playTracks(ids) : nothing());
  const shuffle = () => {
    if (!ids.length) return nothing();
    const order = shuffled(ids);
    void player.playTracks(order, order[0]);
  };
  const queue = () => (tracks.length ? void player.addToQueue(tracks) : nothing());

  const onMissingList = (row: ImportedRow) => {
    missing.current ??= backend.importMissing().then(
      (m) => new Set(m.map((s) => missingKey(s.artist, s.title))),
      () => new Set<string>(),
    );
    return missing.current.then((s) => s.has(missingKey(row.artist, row.title)));
  };

  /** Double-click or Enter: play from here, or say why a dimmed row can't. */
  const activate = async (row: ImportedRow) => {
    if (row.track) {
      void player.playTracks(ids, row.track.id);
      return;
    }
    if (row.status === "unsure") {
      useToasts.getState().push({
        kind: "info",
        message: "Platter isn't sure this song is in your library",
        action: { label: "Check it", run: () => nav.go({ name: "import" }) },
      });
      return;
    }
    const listed = await onMissingList(row);
    useToasts.getState().push({
      kind: "info",
      message: "This song isn't in your library",
      ...(listed ? { action: { label: "See in Missing", run: () => showInMissing(row.artist, row.title) } } : {}),
    });
  };

  const remove = (row: ImportedRow) => {
    backend.importListRemove(list.id, row.songId).then(
      () => {
        setSelected(null);
        importChanged();
        load();
        toast.info(`Removed from ${list.name}`);
      },
      (e) => toast.error(failText(`Couldn't remove ${row.title} from ${list.name}`, e)),
    );
  };

  const removeItem = (row: ImportedRow): MenuItem => ({ label: "Remove from this list", onSelect: () => remove(row) });

  /** A song in the library gets its usual menu; a dimmed one a short one. */
  const menuFor = (row: ImportedRow): MenuItem[] => {
    if (row.track) return trackMenuItems(row.track, { afterPlaylist: [removeItem(row)] });
    const links = storeLinks(row.artist, row.title);
    const open = (url: string) => backend.importOpenLink(url).catch((e) => toast.error(failText(`Couldn't open the link`, e)));
    const items: MenuItem[] = [];
    if (row.status === "unsure") items.push({ label: "Check the match", onSelect: () => nav.go({ name: "import" }) });
    else items.push({ label: "Find in Missing", onSelect: () => showInMissing(row.artist, row.title) });
    STORE_LINKS.forEach((st, i) => items.push({ label: `Search ${st.name}`, icon: null, separator: i === 0, onSelect: () => void open(links[st.key]) }));
    items.push({ ...removeItem(row), separator: true });
    return items;
  };

  const pageMenu = (at: HTMLElement) =>
    openContextMenu(
      `${list.name} options`,
      [
        { label: "Add to queue", onSelect: queue },
        ...(ids.length ? addToPlaylistItems(() => ids) : []),
        { label: list.liked ? "Like matched songs" : "Save as playlist", disabled: !ids.length, onSelect: () => setAsk(true) },
      ],
      at,
    );

  return (
    <>
      <h1 className="page-title">{list.name}</h1>
      <p className="import-page__sub">
        {importSourceName(list.source)} • {listHeaderLine(rows)}
      </p>
      <div className="import-actions">
        {ids.length > 0 && (
          <IconButton label={`Play ${list.name}`} variant="accent" size="xl" text="Play" icon={<Play size={24} fill="currentColor" className="nudge-right" />} onClick={play} />
        )}
        <IconButton label={`Shuffle ${list.name}`} size="lg" icon={<Shuffle size={22} />} onClick={shuffle} />
        <IconButton label={`More options for ${list.name}`} size="lg" icon={<Ellipsis size={22} />} onClick={(e) => pageMenu(e.currentTarget)} />
        <Button disabled={!ids.length} onClick={() => setAsk(true)}>
          {list.liked ? "Like matched songs" : "Save as playlist"}
        </Button>
      </div>
      <ul className="import-rows" aria-label={list.name}>
        <li className="import-row import-row--header" aria-hidden="true">
          <span className="import-row__n">#</span>
          <span>Title</span>
          <span>Album</span>
          <span className="import-row__dur">Length</span>
          <span />
        </li>
        <VirtualRows
          count={rows.length}
          rowHeight={IMPORT_ROW_H}
          renderRow={(i) => {
            const r = rows[i];
            return (
              <Row
                key={`${r.songId}-${i}`}
                row={r}
                index={i}
                selected={selected === i}
                onSelect={() => setSelected(i)}
                onActivate={() => void activate(r)}
                menu={() => menuFor(r)}
              />
            );
          }}
        />
      </ul>
      {ask && <SaveConfirm list={list} tracks={tracks} ids={ids} leftNote={leftNote} onClose={() => setAsk(false)} />}
    </>
  );
}

function SaveConfirm({ list, tracks, ids, leftNote, onClose }: { list: ImportedList; tracks: Track[]; ids: string[]; leftNote: string; onClose: () => void }) {
  return list.liked ? (
    <Confirm
      title="Like these songs?"
      body={`${n(ids.length)} ${ids.length === 1 ? "song" : "songs"} in your library will be liked.${leftNote}`}
      confirm={`Like ${n(ids.length)} ${ids.length === 1 ? "song" : "songs"}`}
      onConfirm={() => void setLiked(tracks, true)}
      onClose={onClose}
    />
  ) : (
    <Confirm
      title="Save as a Plex playlist?"
      body={`A new Plex playlist named "${list.name}" gets the ${n(ids.length)} ${ids.length === 1 ? "song" : "songs"} in your library.${leftNote}`}
      confirm="Save playlist"
      onConfirm={() => void createPlaylist({ trackIds: ids, title: list.name, open: false })}
      onClose={onClose}
    />
  );
}
