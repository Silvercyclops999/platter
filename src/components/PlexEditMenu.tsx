/**
 * The "Edit" submenu on album and artist menus. Shown only for items
 * on servers this account can edit, with editing turned on in Settings.
 */
import { GitMerge, History, ImageIcon, Pencil, RefreshCw, ScanSearch, Split, Unlink, Wallpaper } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { backend } from "../lib/backend";
import type { EditKind } from "../lib/types";
import { canEditNow, runEdit, usePlexEdit, type EditDialog, type EditTarget } from "../stores/plexEdit";
import { useSettings } from "../stores/settings";
import type { MenuItem, SubmenuContext } from "../stores/ui";
import { MenuIcon } from "./ContextMenu";

export function plexEditItem(target: EditTarget): MenuItem {
  return {
    label: "Edit",
    icon: Pencil,
    when: () => canEditNow(target.id),
    submenu: (ctx) => <PlexEditSubmenu target={target} ctx={ctx} />,
  };
}

interface Entry {
  label: string;
  icon: import("lucide-react").LucideIcon;
  run: () => void;
}

export function PlexEditSubmenu({ target, ctx }: { target: EditTarget; ctx: SubmenuContext }) {
  const root = useRef<HTMLDivElement>(null);
  const matchUnreleased = useSettings((s) => s.settings?.plexEditMatchUnreleased !== false);
  // Fix match can be hidden on unreleased albums (Settings).
  const [unreleased, setUnreleased] = useState<boolean | null>(target.kind === "album" ? null : false);
  useEffect(() => {
    if (target.kind !== "album" || matchUnreleased) return;
    let live = true;
    backend.albumReleaseStatus(target.id).then(
      (s) => live && setUnreleased(s.status === "taggedUnreleased" || s.status === "probablyUnreleased"),
      () => live && setUnreleased(false),
    );
    return () => {
      live = false;
    };
  }, [target.id, target.kind, matchUnreleased]);

  const open = (name: EditDialog) => () => usePlexEdit.getState().open(name, target);
  const kind: EditKind = target.kind;
  const art = { path: target.thumb, kind };
  const entries: Entry[] = [
    { label: kind === "album" ? "Change cover…" : "Change photo…", icon: ImageIcon, run: open("cover") },
    ...(kind === "artist" ? [{ label: "Change background…", icon: Wallpaper, run: open("background") }] : []),
    ...(kind === "album" && !matchUnreleased && unreleased !== false ? [] : [{ label: "Fix match…", icon: ScanSearch, run: open("match") }]),
    {
      label: "Unmatch",
      icon: Unlink,
      run: () => void runEdit("Couldn't unmatch", () => backend.plexEditUnmatch(target.id, kind), art),
    },
    { label: "Edit details…", icon: Pencil, run: open("details") },
    ...(kind === "artist"
      ? [
          { label: "Merge with…", icon: GitMerge, run: open("merge") },
          { label: "Split…", icon: Split, run: open("split") },
        ]
      : []),
    {
      label: "Refresh details",
      icon: RefreshCw,
      run: () => void runEdit("Couldn't refresh", () => backend.plexEditRefresh(target.id, kind), art),
    },
    { label: "Recent changes…", icon: History, run: open("history") },
  ];

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const els = Array.from(root.current?.querySelectorAll<HTMLElement>("[role='menuitem']") ?? []);
    const i = els.indexOf(document.activeElement as HTMLElement);
    els[(i + (e.key === "ArrowDown" ? 1 : -1) + els.length) % els.length]?.focus();
  };

  return (
    <div ref={root} className="share-menu" onKeyDown={onKeyDown}>
      {entries.map((x) => (
        <button
          key={x.label}
          type="button"
          role="menuitem"
          tabIndex={-1}
          className="context-menu__item share-menu__item"
          onClick={() => {
            ctx.close();
            x.run();
          }}
        >
          <MenuIcon icon={x.icon} />
          <span className="truncate context-menu__label">{x.label}</span>
        </button>
      ))}
    </div>
  );
}
