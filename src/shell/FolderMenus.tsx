/**
 * Library folder menus and dialogs: the folder menu (rename, delete, create
 * playlist or folder inside, move to folder), the "Move to folder" picker,
 * the rename field and the delete question.
 */
import { Folder, FolderPlus, Library } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import type { LibraryFolders } from "../lib/types";
import { ancestors, folders, useFolders } from "../stores/folders";
import { createPlaylist } from "../stores/playlists";
import type { MenuItem, SubmenuContext } from "../stores/ui";
import "./folders.css";

export type FolderTarget = { kind: "playlist" | "folder"; id: string; title: string };

/** Folders in tree order with their depth (for the picker). */
export function folderOutline(tree: LibraryFolders): { id: string; name: string; depth: number }[] {
  const out: { id: string; name: string; depth: number }[] = [];
  const known = new Set(tree.folders.map((f) => f.id));
  const walk = (parent: string | null, depth: number) => {
    const kids = tree.folders
      .filter((f) => (f.parent && known.has(f.parent) ? f.parent : null) === parent)
      .sort((a, b) => a.name.localeCompare(b.name));
    for (const f of kids) {
      out.push({ id: f.id, name: f.name, depth });
      walk(f.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

/** The "Move to folder" submenu: Your Library, New folder, then every folder. */
export function FolderPicker({ target, ctx }: { target: FolderTarget; ctx: SubmenuContext }) {
  const tree = useFolders((s) => s.tree);
  const root = useRef<HTMLDivElement>(null);
  const current = target.kind === "playlist" ? (tree.items[target.id] ?? null) : (tree.folders.find((f) => f.id === target.id)?.parent ?? null);
  // A folder can't go into itself or a folder inside it.
  const blocked = (id: string) => target.kind === "folder" && (id === target.id || ancestors(tree, id).includes(target.id));
  const list = folderOutline(tree).filter((f) => !blocked(f.id));
  const move = (folderId: string | null) => {
    ctx.close();
    if (target.kind === "playlist") void folders.place(target.id, folderId, target.title);
    else void folders.move(target.id, folderId);
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const els = Array.from(root.current?.querySelectorAll<HTMLElement>("[role='menuitem']:not([aria-disabled='true'])") ?? []);
    const i = els.indexOf(document.activeElement as HTMLElement);
    els[(i + (e.key === "ArrowDown" ? 1 : -1) + els.length) % els.length]?.focus();
  };
  return (
    <div ref={root} className="folder-picker" onKeyDown={onKeyDown}>
      <button
        type="button"
        role="menuitem"
        tabIndex={-1}
        className="context-menu__item"
        aria-disabled={current === null}
        onClick={() => current !== null && move(null)}
      >
        <Library size={16} aria-hidden="true" />
        <span className="truncate">Your Library</span>
      </button>
      <button
        type="button"
        role="menuitem"
        tabIndex={-1}
        className="context-menu__item"
        onClick={async () => {
          ctx.close();
          const f = await folders.create(null);
          if (!f) return;
          if (target.kind === "playlist") await folders.place(target.id, f.id, target.title);
          else await folders.move(target.id, f.id);
          folders.startRename(f.id);
        }}
      >
        <FolderPlus size={16} aria-hidden="true" />
        <span className="truncate">New folder</span>
      </button>
      {list.length > 0 && <div className="context-menu__sep" role="separator" />}
      <div className="folder-picker__list">
        {list.map((f) => (
          <button
            key={f.id}
            type="button"
            role="menuitem"
            tabIndex={-1}
            className="context-menu__item"
            aria-disabled={f.id === current}
            style={{ paddingInlineStart: `${12 + f.depth * 16}px` }}
            onClick={() => f.id !== current && move(f.id)}
          >
            <Folder size={16} aria-hidden="true" />
            <span className="truncate">{f.name}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

export function moveToFolderItem(target: FolderTarget): MenuItem {
  return { label: "Move to folder", icon: Folder, submenu: (ctx) => <FolderPicker target={target} ctx={ctx} /> };
}

/** A new playlist inside a folder (named from Settings, opened for renaming). */
export async function createPlaylistIn(folderId: string | null) {
  const p = await createPlaylist();
  if (p && folderId) await folders.place(p.id, folderId);
}

/** The folder menu (the ⋯ in a drilled-in header, or right-click on a folder row). */
export function folderMenu(id: string): MenuItem[] {
  const f = useFolders.getState().tree.folders.find((x) => x.id === id);
  if (!f) return [];
  return [
    { label: "Rename", onSelect: () => folders.startRename(id) },
    { label: "Delete", onSelect: () => folders.askDelete(id) },
    { label: "Create playlist", separator: true, onSelect: () => void createPlaylistIn(id) },
    {
      label: "Create folder",
      icon: FolderPlus,
      onSelect: async () => {
        const made = await folders.create(id);
        if (made) folders.startRename(made.id);
      },
    },
    moveToFolderItem({ kind: "folder", id, title: f.name }),
  ];
}

function RenameDialog() {
  const id = useFolders((s) => s.renaming);
  const f = useFolders((s) => s.tree.folders.find((x) => x.id === id));
  const [name, setName] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (f) setName(f.name);
    // The name is selected, ready to type over.
    const t = setTimeout(() => input.current?.select(), 0);
    return () => clearTimeout(t);
  }, [id]);
  if (!id || !f) return null;
  const close = () => useFolders.setState({ renaming: null });
  const save = () => {
    const n = name.trim();
    close();
    if (n && n !== f.name) void folders.rename(id, n);
  };
  return (
    <Dialog open onClose={close} label="Rename folder" className="confirm folder-rename" initialFocus={input}>
      <h2 className="confirm__title">Rename folder</h2>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <input
          ref={input}
          className="folder-rename__input"
          aria-label="Folder name"
          value={name}
          maxLength={100}
          spellCheck={false}
          onChange={(e) => setName(e.target.value)}
        />
        <div className="confirm__actions">
          <Button type="button" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={!name.trim()}>
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function DeleteDialog() {
  const id = useFolders((s) => s.deleting);
  const tree = useFolders((s) => s.tree);
  const cancel = useRef<HTMLButtonElement>(null);
  const f = tree.folders.find((x) => x.id === id);
  if (!id || !f) return null;
  const close = () => useFolders.setState({ deleting: null });
  const inside = Object.values(tree.items).filter((x) => x === id).length + tree.folders.filter((x) => x.parent === id).length;
  const where = f.parent ? `"${tree.folders.find((x) => x.id === f.parent)?.name ?? "the folder above"}"` : "Your Library";
  return (
    <Dialog open onClose={close} label={`Delete ${f.name}?`} className="confirm" initialFocus={cancel}>
      <h2 className="confirm__title">Delete this folder?</h2>
      <p className="confirm__body">
        {inside > 0
          ? `What's in "${f.name}" moves to ${where}. Your playlists stay in Plex; only the folder goes.`
          : `"${f.name}" is empty. Only the folder goes.`}
      </p>
      <div className="confirm__actions">
        <Button ref={cancel} variant="ghost" onClick={close}>
          Cancel
        </Button>
        <Button
          variant="primary"
          className="btn--danger"
          onClick={() => {
            close();
            void folders.remove(id);
          }}
        >
          Delete
        </Button>
      </div>
    </Dialog>
  );
}

export function FolderDialogs() {
  return (
    <>
      <RenameDialog />
      <DeleteDialog />
    </>
  );
}
