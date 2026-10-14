import { Play, Search as SearchIcon, X } from "lucide-react";
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { Art } from "../components/Art";
import { SearchField } from "../components/SearchField";
import { Tooltip } from "../components/Tooltip";
import { backend } from "../lib/backend";
import { modKey } from "../lib/platform";
import type { SearchHit } from "../lib/types";
import { nav, useCurrentEntry, useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { dropdownHits, hitInfo, openHit, playHit, rememberHit, subtitleWithType, typedPrefixLength } from "./searchModel";

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
        setCompletions(c);
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

  // A click anywhere outside the box and its dropdown closes the dropdown.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  // Keep the highlight on screen.
  useEffect(() => {
    if (sel < 0) return;
    document.getElementById(`${listId}-${sel}`)?.scrollIntoView?.({ block: "nearest" });
  }, [sel, listId]);

  if (collapsed && !expanded)
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
      className={`searchbox ${collapsed ? "searchbox--expanded" : ""} ${shown ? "is-open" : ""}`}
      ref={root}
      onBlur={(e) => {
        if (root.current?.contains(e.relatedTarget as Node)) return;
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
              <kbd>↓</kbd> to move · <kbd>Enter</kbd> to search
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
              const info = hitInfo(row.hit);
              if (!info) return null;
              return (
                <li
                  key={`h:${info.key}`}
                  id={id}
                  role="option"
                  aria-selected={sel === i}
                  aria-label={`${info.title}, ${subtitleWithType(info)}`}
                  className={cls}
                  onClick={() => activate(row)}
                >
                  <Art path={info.thumb} size={40} round={info.round} kind={info.kind === "artist" ? "artist" : "album"} />
                  <span className="searchbox__text">
                    <span className="searchbox__title truncate">{info.title}</span>
                    <span className="searchbox__sub truncate">{subtitleWithType(info)}</span>
                  </span>
                  <button
                    type="button"
                    tabIndex={-1}
                    className="searchbox__play"
                    aria-label={`Play ${info.title}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      rememberHit(row.hit);
                      void playHit(row.hit);
                    }}
                  >
                    <Play size={16} fill="currentColor" aria-hidden="true" />
                  </button>
                  {row.recent && (
                    <button
                      type="button"
                      tabIndex={-1}
                      className="searchbox__remove"
                      aria-label={`Remove ${info.title} from recent searches`}
                      onClick={(e) => {
                        e.stopPropagation();
                        removeRecent(row.hit);
                      }}
                    >
                      <X size={16} aria-hidden="true" />
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
