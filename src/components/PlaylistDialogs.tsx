/**
 * Playlist dialogs, mounted once in the shell: Already added, Remove songs,
 * Delete playlist and Edit details. Which one is open lives in
 * stores/playlists.
 */
import { ImagePlus } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { backend } from "../lib/backend";
import { coverImage } from "../lib/coverImage";
import type { Playlist } from "../lib/types";
import { answerDuplicate, confirmDelete, confirmRemove, savePlaylistDetails, usePlaylistUi } from "../stores/playlists";
import { toast } from "../stores/toasts";
import { Art } from "./Art";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import "./playlist-dialogs.css";

export const NAME_MAX = 100;
export const DESCRIPTION_MAX = 300;

export function PlaylistDialogs() {
  return (
    <>
      <DuplicateDialog />
      <RemoveDialog />
      <DeleteDialog />
      <EditDetailsHost />
    </>
  );
}

function DuplicateDialog() {
  const ask = usePlaylistUi((s) => s.duplicate);
  const dontAdd = useRef<HTMLButtonElement>(null);
  if (!ask) return null;
  const several = ask.trackIds.length > 1;
  const someNew = ask.trackIds.some((t) => !ask.duplicates.includes(t));
  const body = several
    ? ask.duplicates.length === ask.trackIds.length
      ? `These songs are already in "${ask.playlist.title}".`
      : `Some of these songs are already in "${ask.playlist.title}".`
    : `This song is already in "${ask.playlist.title}".`;
  return (
    <Dialog open onClose={() => answerDuplicate("none")} label="Already added" className="confirm" initialFocus={dontAdd}>
      <h2 className="confirm__title">Already added</h2>
      <p className="confirm__body">{body}</p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={() => answerDuplicate("all")}>
          Add anyway
        </Button>
        {several && someNew && (
          <Button variant="secondary" onClick={() => answerDuplicate("new")}>
            Add new ones
          </Button>
        )}
        <Button ref={dontAdd} variant="primary" onClick={() => answerDuplicate("none")}>
          Don't add
        </Button>
      </div>
    </Dialog>
  );
}

function RemoveDialog() {
  const ask = usePlaylistUi((s) => s.removing);
  const remove = useRef<HTMLButtonElement>(null);
  if (!ask) return null;
  const n = ask.itemIds.length;
  const title = n === 1 ? "Remove 1 song?" : `Remove ${n} songs?`;
  return (
    <Dialog open onClose={() => usePlaylistUi.getState().set({ removing: null })} label={title} className="confirm" initialFocus={remove}>
      <h2 className="confirm__title">{title}</h2>
      <p className="confirm__body">
        {n === 1 ? "It comes out" : "They come out"} of "{ask.playlist.title}". You'll need to add {n === 1 ? "it" : "them"} again if you change your mind.
      </p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={() => usePlaylistUi.getState().set({ removing: null })}>
          Cancel
        </Button>
        <Button ref={remove} variant="primary" onClick={() => void confirmRemove()}>
          Remove
        </Button>
      </div>
    </Dialog>
  );
}

function DeleteDialog() {
  const p = usePlaylistUi((s) => s.deleting);
  if (!p) return null;
  return (
    <Dialog open onClose={() => usePlaylistUi.getState().set({ deleting: null })} label={`Delete ${p.title}?`} className="confirm">
      <h2 className="confirm__title">Delete this playlist?</h2>
      <p className="confirm__body">"{p.title}" will be deleted from Plex, so it's gone from your other Plex apps too.</p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={() => usePlaylistUi.getState().set({ deleting: null })}>
          Cancel
        </Button>
        <Button variant="primary" className="btn--danger" onClick={() => void confirmDelete()}>
          Delete
        </Button>
      </div>
    </Dialog>
  );
}

function EditDetailsHost() {
  const p = usePlaylistUi((s) => s.editing);
  if (!p) return null;
  return <EditDetailsDialog key={p.id} playlist={p} onClose={() => usePlaylistUi.getState().set({ editing: null })} />;
}

/** Cover, name (up to 100) and description (up to 300), saved together. */
export function EditDetailsDialog({ playlist, onClose }: { playlist: Playlist; onClose: () => void }) {
  const id = useId();
  const [title, setTitle] = useState(playlist.title);
  const [summary, setSummary] = useState(playlist.summary ?? "");
  const [cover, setCover] = useState<{ bytes: Uint8Array; type: string; url: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => () => {
    if (cover) URL.revokeObjectURL(cover.url);
  }, [cover]);

  const pickFile = async (file: File | undefined) => {
    if (!file) return;
    try {
      const img = await coverImage(file);
      setCover({ ...img, url: URL.createObjectURL(new Blob([img.bytes as BlobPart], { type: img.type })) });
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't read that image.");
    }
  };

  const save = async () => {
    const name = title.trim();
    if (!name) {
      setError("Playlist name is required.");
      nameRef.current?.focus();
      return;
    }
    setSaving(true);
    const details: { title?: string; summary?: string } = {};
    if (name !== playlist.title) details.title = name;
    if (summary.trim() !== (playlist.summary ?? "")) details.summary = summary.trim();
    let ok = true;
    if (details.title !== undefined || details.summary !== undefined) ok = await savePlaylistDetails(playlist, details);
    if (ok && cover) {
      try {
        await backend.playlistSetCover(playlist.id, cover.bytes, cover.type);
      } catch (e) {
        ok = false;
        const m = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
        toast.error(m ? `Couldn't change the cover: ${m}` : "Couldn't change the cover");
      }
    }
    setSaving(false);
    if (ok) onClose();
  };

  return (
    <Dialog open onClose={onClose} label="Edit details" className="pl-edit" initialFocus={nameRef}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <h2 className="pl-edit__title">Edit details</h2>
        <div className="pl-edit__body">
          <button type="button" className="pl-edit__cover" aria-label="Choose a cover image" onClick={() => fileRef.current?.click()}>
            {cover ? <img src={cover.url} alt="" width={180} height={180} /> : <Art path={playlist.thumb} size={180} kind="playlist" />}
            <span className="pl-edit__cover-hint" aria-hidden="true">
              <ImagePlus size={32} />
              <span>Choose image</span>
            </span>
          </button>
          <input
            ref={fileRef}
            type="file"
            accept="image/jpeg,image/png,image/webp"
            hidden
            data-testid="cover-file"
            onChange={(e) => void pickFile(e.target.files?.[0])}
          />
          <div className="pl-edit__fields">
            <label className="pl-edit__field" htmlFor={`${id}-name`}>
              <span className="pl-edit__label">Name</span>
              <input
                ref={nameRef}
                id={`${id}-name`}
                className="pl-edit__input"
                value={title}
                maxLength={NAME_MAX}
                aria-describedby={`${id}-name-count`}
                onChange={(e) => {
                  setTitle(e.target.value);
                  setError(null);
                }}
              />
              <span id={`${id}-name-count`} className="pl-edit__count" aria-label={`${[...title].length} of ${NAME_MAX} characters`}>
                {[...title].length}/{NAME_MAX}
              </span>
            </label>
            <label className="pl-edit__field pl-edit__field--grow" htmlFor={`${id}-desc`}>
              <span className="pl-edit__label">Description</span>
              <textarea
                id={`${id}-desc`}
                className="pl-edit__input pl-edit__textarea"
                value={summary}
                maxLength={DESCRIPTION_MAX}
                placeholder="Add an optional description"
                aria-describedby={`${id}-desc-count`}
                onChange={(e) => setSummary(e.target.value)}
              />
              <span id={`${id}-desc-count`} className="pl-edit__count" aria-label={`${[...summary].length} of ${DESCRIPTION_MAX} characters`}>
                {[...summary].length}/{DESCRIPTION_MAX}
              </span>
            </label>
          </div>
        </div>
        {error && (
          <p className="confirm__error" role="alert">
            {error}
          </p>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" disabled={saving}>
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
