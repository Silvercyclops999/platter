import { MoreHorizontal, Pause, Play, Search as SearchIcon, X } from "lucide-react";
import { useEffect, useId, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { Art } from "../components/Art";
import { openContextMenu } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { SearchField } from "../components/SearchField";
import { Tooltip } from "../components/Tooltip";
import { backend } from "../lib/backend";
import { modKey } from "../lib/platform";
import type { SearchHit } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { nav, useCurrentEntry, useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { QueueButton } from "./SearchActions";
import {
  dropdownHits,
  hitInfo,
  hitMenuItems,
  openHit,
  playHit,
  queueHit,
  rememberHit,
  subtitleWithType,
  typedPrefixLength,
  uniqueCompletions,
} from "./searchModel";

/** Longest query sent to the core; anything past it is ignored. */
export const MAX_QUERY = 200;

type Row =
  | { kind: "completion"; text: string }
  | { kind: "hit"; hit: SearchHit; recent: boolean };

/** Open the full results page for a query (a new history entry). */
export function openResults(query: string) {
  const q = query.trim().slice(0, MAX_QUERY);
  const { entries, index } = useRouter.getState();
  const r = entries[index].route;
  if (r.name === "search" && !r.query) useRouter.getState().replace({ name: "search", query: q });
  else nav.go({ name: "search", query: q });
}

/** Popups that open from the dropdown (a row's menu, the like control's "saved in") and keep it open. */
const POPUPS = ".context-menu, .saved-in";
const inPopup = (t: EventTarget | null) => t instanceof Element && !!t.closest(POPUPS);

/** Whether a song is the one playing (its play button pauses instead). */
function useSongPlaying(id: string | undefined) {
  return usePlayer((s) => !!id && s.state?.track?.id === id && s.state?.status === "playing");
}

function Completion({ text, typed }: { text: string; typed: string }) {
  const n = typedPrefixLength(text, typed);
  return (
    <span className="truncate">
      <strong>{text.slice(0, n)}</strong>
      {text.slice(n)}
    </span>
  );
}

/**
 * One direct result in the dropdown. Songs get play on the cover, add to
 * queue, like and more options; the rest a play button. Right-click opens
 * the same menu as elsewhere in the app.
 */
function HitRow({
  id,
  hit,
  recent,
  selected,
  className,
  onActivate,
  onMenu,
  onRemove,
}: {
  id: string;
  hit: SearchHit;
  recent: boolean;
  selected: boolean;
  className: string;
  onActivate: () => void;
  onMenu: (at: { clientX: number; clientY: number } | HTMLElement) => void;
  onRemove: () => void;
}) {
  const info = hitInfo(hit)!;
  const song = hit.kind === "track" ? hit.track : undefined;
  const playing = useSongPlaying(song?.id);
  const more = useRef<HTMLButtonElement>(null);
  const play = (e: MouseEvent) => {
    e.stopPropagation();
    rememberHit(hit);
    if (song && usePlayer.getState().state?.track?.id === song.id) void player.toggle();
    else void playHit(hit);
  };
  const playLabel = `${playing ? "Pause" : "Play"} ${info.title}`;
  const PlayIcon = playing ? Pause : Play;
  return (
    <li
      id={id}
      role="option"
      aria-selected={selected}
      aria-label={`${info.title}, ${subtitleWithType(info)}`}
      className={`${className} ${song ? "searchbox__row--song" : ""}`}
      onClick={onActivate}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(e);
      }}
    >
      {song ? (
        <span className="searchbox__art">
          <Art path={info.thumb} size={40} kind="album" />
          <button type="button" tabIndex={-1} className="searchbox__art-play" aria-label={playLabel} onClick={play}>
            <PlayIcon size={16} fill="currentColor" aria-hidden="true" />
          </button>
        </span>
      ) : (
        <Art path={info.thumb} size={40} round={info.round} kind={info.kind === "artist" ? "artist" : "album"} />
      )}
      <span className="searchbox__text">
        <span className="searchbox__title truncate">{info.title}</span>
        <span className="searchbox__sub truncate">{subtitleWithType(info)}</span>
      </span>
      {song ? (
        <span className="searchbox__actions">
          <QueueButton hit={hit} className="searchbox__action" />
          <SaveButton track={song} place="row" />
          <button
            ref={more}
            type="button"
            tabIndex={-1}
            className="searchbox__action"
            aria-label={`More options for ${info.title}`}
            onClick={(e) => {
              e.stopPropagation();
              onMenu(more.current!);
            }}
          >
            <MoreHorizontal size={18} aria-hidden="true" />
          </button>
        </span>
      ) : (
        <button type="button" tabIndex={-1} className="searchbox__play" aria-label={playLabel} onClick={play}>
          <Play size={16} fill="currentColor" aria-hidden="true" />
        </button>
      )}
      {recent && (
        <button
          type="button"
          tabIndex={-1}
          className="searchbox__remove"
          aria-label={`Remove ${info.title} from recent searches`}
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
        >
          <X size={16} aria-hidden="true" />
        </button>
      )}
    </li>
  );
}

/**
 * The top bar's search pill and its dropdown: recent searches when the box
 * is empty, completions and direct results while typing. Focus stays in the
 * box; arrows move a highlight without changing the text.
 */
export function SearchBox({ collapsed = false }: { collapsed?: boolean }) {
  const entry = useCurrentEntry();
  const routeQuery = entry.route.name === "search" ? (entry.route.query ?? "") : null;
  const suggestOn = useSettings((s) => s.settings?.searchSuggestions !== false);
  const recentsOn = useSettings((s) => s.settings?.searchRecents !== false);
  const tick = useUi((s) => s.searchFocusTick);
  const [text, setText] = useState(routeQuery ?? "");
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [sel, setSel] = useState(-1);
  const [recents, setRecents] = useState<SearchHit[] | null>(null);
  const [completions, setCompletions] = useState<string[]>([]);
  const [hits, setHits] = useState<SearchHit[]>([]);
  const input = useRef<HTMLInputElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const seq = useRef(0);
  // Set while a click outside closes a row's menu, which hands focus back to
  // the box: that focus shouldn't reopen the dropdown.
  const quietFocus = useRef(false);
  const listId = useId();

  // The box shows the results page's query when you arrive on one.
  useEffect(() => {
    if (routeQuery !== null) setText(routeQuery);
    setOpen(false);
    setSel(-1);
  }, [entry.key, routeQuery]);

  // Ctrl/⌘+L (or the palette) focuses the box from anywhere.
  useEffect(() => {
    if (tick === 0) return;
    if (collapsed) setExpanded(true);
    requestAnimationFrame(() => {
      input.current?.focus();
      input.current?.select();
    });
  }, [tick, collapsed]);

  const q = text.trim().slice(0, MAX_QUERY);

  const loadRecents = () => {
    if (!recentsOn) return setRecents([]);
    backend.searchRecents().then(setRecents, () => setRecents([]));
  };

  // Live suggestions. Only the newest request may update the dropdown.
  useEffect(() => {
    const mine = ++seq.current;
    setSel(-1);
    if (!open || !q || !suggestOn) {
      setCompletions([]);
      setHits([]);
      return;
    }
    Promise.all([backend.searchCompletions(q, 4), backend.search({ query: q, filter: "all", limit: 4 })]).then(
      ([c, res]) => {
        if (mine !== seq.current) return;
        // A corrected spelling leads the suggestions; the results already follow it.
        setCompletions(uniqueCompletions(res.corrected ? [res.corrected, ...c] : c).slice(0, 4));
        setHits(dropdownHits(res));
      },
      () => {
        if (mine !== seq.current) return;
        setCompletions([]);
        setHits([]);
      },
    );
  }, [q, open, suggestOn]);

  const rows: Row[] = !q
    ? recentsOn
      ? (recents ?? []).map((hit) => ({ kind: "hit" as const, hit, recent: true }))
      : []
    : suggestOn
      ? [
          ...completions.map((text) => ({ kind: "completion" as const, text })),
          ...hits.map((hit) => ({ kind: "hit" as const, hit, recent: false })),
        ]
      : [];
  const shown = open && rows.length > 0;

  const close = () => {
    setOpen(false);
    setSel(-1);
  };

  const activate = (row: Row) => {
    if (row.kind === "completion") {
      setText(row.text);
      close();
      openResults(row.text);
      return;
    }
    rememberHit(row.hit);
    close();
    input.current?.blur();
    openHit(row.hit);
  };

  const removeRecent = (hit: SearchHit) => {
    const i = hitInfo(hit);
    if (!i) return;
    setRecents((r) => r?.filter((x) => hitInfo(x)?.key !== i.key) ?? null);
    backend.searchRecentRemove(i.kind, i.id).catch(() => loadRecents());
  };

  // A row's menu opens over the dropdown and hands focus back to the box.
  const openRowMenu = (hit: SearchHit, at: { clientX: number; clientY: number } | HTMLElement) => {
    const i = hitInfo(hit);
    if (!i) return;
    const s = usePlayer.getState().state;
    const current = i.kind === "track" && s?.track?.id === i.id;
    const toggle = () => {
      rememberHit(hit);
      if (current) void player.toggle();
      else void playHit(hit);
    };
    openContextMenu(`Options for ${i.title}`, hitMenuItems(hit, current && s?.status === "playing", toggle), at, input.current);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) {
        setOpen(true);
        if (!q) loadRecents();
        return;
      }
      if (!rows.length) return;
      const last = rows.length - 1;
      setSel((s) => (e.key === "ArrowDown" ? Math.min(s + 1, last) : Math.max(s - 1, -1)));
    } else if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
      // The highlighted result's menu.
      const row = shown && sel >= 0 ? rows[sel] : undefined;
      const el = document.getElementById(`${listId}-${sel}`);
      if (row?.kind !== "hit" || !el) return;
      e.preventDefault();
      openRowMenu(row.hit, el);
    } else if (e.key === "Enter" && e.shiftKey) {
      // Shift+Enter adds the highlighted song to the queue; the dropdown stays.
      e.preventDefault();
      const row = shown && sel >= 0 ? rows[sel] : undefined;
      if (row?.kind === "hit") void queueHit(row.hit);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (shown && sel >= 0 && rows[sel]) activate(rows[sel]);
      else if (q) {
        close();
        openResults(q);
      }
    } else if (e.key === "Escape") {
      // First Escape closes the dropdown and keeps the text; the next clears it.
      e.preventDefault();
      e.stopPropagation();
      if (shown) close();
      else if (text) setText("");
      else if (collapsed) {
        setExpanded(false);
        input.current?.blur();
      }
    }
  };

  // A click anywhere outside the box and its dropdown closes the dropdown
  // (but not one in a menu opened from it).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (root.current?.contains(e.target as Node) || inPopup(e.target)) return;
      if (useUi.getState().menu) {
        quietFocus.current = true;
        setTimeout(() => (quietFocus.current = false));
      }
      close();
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  // Keep the highlight on screen.
  useEffect(() => {
    if (sel < 0) return;
    document.getElementById(`${listId}-${sel}`)?.scrollIntoView?.({ block: "nearest" });
  }, [sel, listId]);

  // Narrow windows show a round button, but the field stays open while it holds a query.
  if (collapsed && !expanded && !text)
    return (
      <Tooltip label={`Search (${modKey}+L)`} placement="bottom">
        <button
          type="button"
          className="topbar__round"
          aria-label="Search"
          onClick={() => {
            setExpanded(true);
            requestAnimationFrame(() => input.current?.focus());
          }}
        >
          <SearchIcon size={20} aria-hidden="true" />
        </button>
      </Tooltip>
    );

  const activeId = shown && sel >= 0 ? `${listId}-${sel}` : undefined;
  const firstHit = rows.findIndex((r) => r.kind === "hit");

  return (
    <div
      role="search"
      className={`searchbox ${collapsed ? "searchbox--expanded" : ""} ${shown ? "is-open" : ""}`}
      ref={root}
      onBlur={(e) => {
        // A row's menu takes focus while it's open; the dropdown stays behind it.
        if (root.current?.contains(e.relatedTarget as Node) || inPopup(e.relatedTarget) || useUi.getState().menu) return;
        close();
        if (collapsed && !text) setExpanded(false);
      }}
    >
      <SearchField
        ref={input}
        value={text}
        label="Search"
        placeholder="What do you want to play?"
        className="searchbox__field"
        onChange={(v) => {
          setText(v);
          setOpen(true);
          if (!v.trim()) loadRecents();
        }}
        onKeyDown={onKeyDown}
        inputProps={{
          role: "combobox",
          "aria-expanded": shown,
          "aria-controls": listId,
          "aria-autocomplete": "list",
          "aria-activedescendant": activeId,
          onFocus: () => {
            if (quietFocus.current) return;
            setOpen(true);
            if (!q) loadRecents();
          },
          onClick: () => {
            if (!open) {
              setOpen(true);
              if (!q) loadRecents();
            }
          },
        }}
      />
      {shown && (
        // Mouse down here would move focus out of the box; keep it in.
        <div className="searchbox__panel" onMouseDown={(e) => e.preventDefault()}>
          {!q ? (
            <div className="searchbox__head">Recent searches</div>
          ) : (
            <div className="searchbox__hint" aria-hidden="true">
              <kbd>↑</kbd>
              <kbd>↓</kbd> to move · <kbd>Enter</kbd> to search · <kbd>Shift</kbd>
              <kbd>Enter</kbd> to add a song to the queue
            </div>
          )}
          <ul className="searchbox__list" role="listbox" id={listId} aria-label={q ? "Suggestions" : "Recent searches"}>
            {rows.map((row, i) => {
              const id = `${listId}-${i}`;
              const cls = `searchbox__row ${sel === i ? "is-selected" : ""} ${i === firstHit && i > 0 ? "has-rule" : ""}`;
              if (row.kind === "completion")
                return (
                  <li key={`c:${row.text}`} id={id} role="option" aria-selected={sel === i} className={`${cls} searchbox__row--completion`} onClick={() => activate(row)}>
                    <span className="searchbox__icon" aria-hidden="true">
                      <SearchIcon size={18} />
                    </span>
                    <Completion text={row.text} typed={q} />
                  </li>
                );
              if (!hitInfo(row.hit)) return null;
              return (
                <HitRow
                  key={`h:${hitInfo(row.hit)!.key}`}
                  id={id}
                  hit={row.hit}
                  recent={row.recent}
                  selected={sel === i}
                  className={cls}
                  onActivate={() => activate(row)}
                  onMenu={(at) => {
                    setSel(i);
                    openRowMenu(row.hit, at);
                  }}
                  onRemove={() => removeRecent(row.hit)}
                />
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
