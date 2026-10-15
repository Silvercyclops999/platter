import { CornerDownLeft, Disc3, History, House, Keyboard, LibraryBig, ListMusic, Music, Repeat, Search, Settings, Shuffle, User, Volume1, Volume2 } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { Dialog } from "../components/Dialog";
import { SearchField } from "../components/SearchField";
import { backend } from "../lib/backend";
import { trackArtistLine } from "../lib/format";
import { modKey } from "../lib/platform";
import type { SearchResults } from "../lib/types";
import { useSettings } from "../stores/settings";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { useUi } from "../stores/ui";
import { shortcutLabel } from "./shortcuts";
import { VOLUME_KEY_STEP } from "./useShortcuts";

interface Cmd {
  id: string;
  label: string;
  hint?: string;
  icon: ReactNode;
  group: string;
  run: () => void;
}

const NAV_COMMANDS: Cmd[] = [
  { id: "go-home", label: "Go to Home", hint: shortcutLabel("home"), icon: <House size={16} />, group: "Navigation", run: () => nav.go({ name: "home" }) },
  { id: "go-search", label: "Go to Search", hint: `${modKey}+L`, icon: <Search size={16} />, group: "Navigation", run: () => useUi.getState().focusSearch() },
  { id: "go-albums", label: "Library: Albums", icon: <LibraryBig size={16} />, group: "Navigation", run: () => nav.go({ name: "library", tab: "albums" }) },
  { id: "go-artists", label: "Library: Artists", icon: <LibraryBig size={16} />, group: "Navigation", run: () => nav.go({ name: "library", tab: "artists" }) },
  { id: "go-playlists", label: "Library: Playlists", icon: <LibraryBig size={16} />, group: "Navigation", run: () => nav.go({ name: "library", tab: "playlists" }) },
  { id: "go-recents", label: "Go to Recents", icon: <History size={16} />, group: "Navigation", run: () => nav.go({ name: "recents" }) },
  { id: "go-settings", label: "Open settings", hint: `${modKey}+,`, icon: <Settings size={16} />, group: "Navigation", run: () => nav.go({ name: "settings" }) },
  { id: "go-back", label: "Go back", hint: shortcutLabel("back"), icon: <CornerDownLeft size={16} />, group: "Navigation", run: () => nav.back() },
  { id: "shortcuts", label: "Keyboard shortcuts", hint: shortcutLabel("help"), icon: <Keyboard size={16} />, group: "Navigation", run: () => useUi.getState().setShortcutsOpen(true) },
  { id: "toggle-queue", label: "Toggle queue", hint: shortcutLabel("queue"), icon: <ListMusic size={16} />, group: "Playback", run: () => useUi.getState().toggleQueue() },
  { id: "play-pause", label: "Play / pause", hint: "Space", icon: <Music size={16} />, group: "Playback", run: () => player.toggle() },
  { id: "next", label: "Next track", hint: `${modKey}+→`, icon: <Music size={16} />, group: "Playback", run: () => player.next() },
  { id: "previous", label: "Previous track", hint: `${modKey}+←`, icon: <Music size={16} />, group: "Playback", run: () => player.previous() },
  { id: "shuffle", label: "Shuffle on / off", hint: `${modKey}+S`, icon: <Shuffle size={16} />, group: "Playback", run: () => player.toggleShuffle(!!useSettings.getState().settings?.smartShuffle) },
  { id: "repeat", label: "Repeat: off, all, one", hint: `${modKey}+R`, icon: <Repeat size={16} />, group: "Playback", run: () => player.cycleRepeatNow() },
  { id: "volume-up", label: "Volume up", hint: `${modKey}+↑`, icon: <Volume2 size={16} />, group: "Playback", run: () => player.nudgeVolume(VOLUME_KEY_STEP) },
  { id: "volume-down", label: "Volume down", hint: `${modKey}+↓`, icon: <Volume1 size={16} />, group: "Playback", run: () => player.nudgeVolume(-VOLUME_KEY_STEP) },
];

/** Ctrl/⌘+K command palette (stub): navigation commands + quick library search. */
export function CommandPalette() {
  const open = useUi((s) => s.paletteOpen);
  const setOpen = useUi((s) => s.setPaletteOpen);
  const [q, setQ] = useState("");
  const [results, setResults] = useState<SearchResults | null>(null);
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const listId = useId();

  useEffect(() => {
    if (!open) {
      setQ("");
      setResults(null);
      setActive(0);
    }
  }, [open]);

  useEffect(() => {
    const query = q.trim();
    if (query.length < 2) {
      setResults(null);
      return;
    }
    let alive = true;
    const t = setTimeout(() => {
      backend.search({ query, filter: "all", limit: 4 }).then((r) => alive && setResults(r), () => {});
    }, 80);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [q]);

  const commands = useMemo(() => {
    const ql = q.trim().toLowerCase();
    const navs = NAV_COMMANDS.filter((c) => !ql || c.label.toLowerCase().includes(ql));
    const found: Cmd[] = [];
    if (results) {
      results.artists.slice(0, 3).forEach((a) =>
        found.push({ id: `ar-${a.id}`, label: a.title, hint: "Artist", icon: <User size={16} />, group: "Library", run: () => nav.go({ name: "artist", id: a.id }) }),
      );
      results.albums.slice(0, 4).forEach((a) =>
        found.push({ id: `al-${a.id}`, label: a.title, hint: a.artistTitle, icon: <Disc3 size={16} />, group: "Library", run: () => nav.go({ name: "album", id: a.id }) }),
      );
      results.songs.slice(0, 4).forEach((t) =>
        found.push({ id: `tr-${t.id}`, label: t.title, hint: trackArtistLine(t), icon: <Music size={16} />, group: "Library", run: () => player.playTracks([t.id], t.id, "search") }),
      );
    }
    const all = [...found, ...navs];
    if (q.trim()) all.push({ id: "search-all", label: `Search for “${q.trim()}”`, icon: <Search size={16} />, group: "Search", run: () => nav.go({ name: "search", query: q.trim() }) });
    return all;
  }, [q, results]);

  useEffect(() => setActive(0), [commands.length]);

  const run = (c: Cmd | undefined) => {
    if (!c) return;
    setOpen(false);
    // Let the dialog restore focus first, then act.
    setTimeout(c.run, 0);
  };

  let lastGroup = "";
  return (
    <Dialog open={open} onClose={() => setOpen(false)} label="Command palette" className="palette" placement="top" initialFocus={input}>
      <div className="palette__input">
        <SearchField
          ref={input}
          value={q}
          onChange={setQ}
          label="Type a command or search"
          placeholder="Type a command or search your library"
          inputProps={{
            role: "combobox",
            "aria-expanded": true,
            "aria-controls": listId,
            "aria-activedescendant": commands[active] ? `${listId}-${commands[active].id}` : undefined,
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setActive((a) => Math.min(commands.length - 1, a + 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setActive((a) => Math.max(0, a - 1));
            } else if (e.key === "Enter") {
              e.preventDefault();
              run(commands[active]);
            }
          }}
        />
      </div>
      <div id={listId} role="listbox" aria-label="Commands" className="palette__list">
        {commands.length === 0 && <div className="palette__empty">No matches</div>}
        {commands.map((c, i) => {
          const header = c.group !== lastGroup ? c.group : null;
          lastGroup = c.group;
          return (
            <div key={c.id} role="none">
              {header && (
                <div className="palette__group" role="presentation">
                  {header}
                </div>
              )}
              <div
                id={`${listId}-${c.id}`}
                role="option"
                aria-selected={i === active}
                className={`palette__item ${i === active ? "is-active" : ""}`}
                onMouseMove={() => setActive(i)}
                onClick={() => run(c)}
              >
                <span className="palette__icon" aria-hidden="true">
                  {c.icon}
                </span>
                <span className="palette__label truncate">{c.label}</span>
                {c.hint && <span className="palette__hint truncate">{c.hint}</span>}
              </div>
            </div>
          );
        })}
      </div>
      <div className="palette__foot" aria-hidden="true">
        <span>↑↓ to move</span>
        <span>↵ to run</span>
        <span>Esc to close</span>
      </div>
    </Dialog>
  );
}
