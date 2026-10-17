import { Bookmark as BookmarkIcon, X } from "lucide-react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { formatDuration } from "../lib/format";
import type { Bookmark } from "../lib/types";
import { openBookmark, removeBookmark, useCurate } from "../stores/curate";
import "../components/smart-editor.css";

const PAGE_LABEL: Record<string, string> = {
  album: "Album",
  artist: "Artist",
  playlist: "Playlist",
  smart: "Smart playlist",
  mix: "Mix",
  radio: "Radio",
  liked: "Liked Songs",
  discography: "Discography",
};

function when(at: number): string {
  return new Date(at * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function Row({ b }: { b: Bookmark }) {
  const song = b.kind === "track";
  const sub = song ? (b.subtitle ?? "") : [PAGE_LABEL[b.page ?? ""] ?? "Page", b.subtitle].filter(Boolean).join(" • ");
  const label = song ? `Play ${b.title} from ${formatDuration(b.positionMs ?? 0)}` : `Open ${b.title}`;
  return (
    <li className="bookmark-row">
      <button type="button" className="bookmark-row__open" aria-label={label} onClick={() => void openBookmark(b)}>
        <Art path={b.thumb ?? null} size={48} round={b.page === "artist"} kind={b.page === "artist" ? "artist" : "album"} />
        <span className="bookmark-row__text">
          <span className="bookmark-row__title truncate">{b.title}</span>
          <span className="bookmark-row__sub truncate">{sub}</span>
        </span>
        <span className="bookmark-row__at">{song ? `at ${formatDuration(b.positionMs ?? 0)}` : when(b.createdAt)}</span>
      </button>
      <IconButton label={`Remove ${b.title}`} size="sm" icon={<X size={16} />} onClick={() => void removeBookmark(b.id)} />
    </li>
  );
}

/** Songs at a moment and pages you bookmarked, newest first. */
export function BookmarksPage() {
  const all = useCurate((s) => s.bookmarks);
  return (
    <div className="page-pad bookmarks" data-testid="bookmarks-page">
      <h1 className="page-title">Bookmarks</h1>
      {all.length === 0 ? (
        <p className="bookmarks__empty" role="status">
          <BookmarkIcon size={16} aria-hidden="true" /> Bookmark a song at the moment you're at (Alt+Shift+M), or any album, artist, playlist or mix from its menu.
        </p>
      ) : (
        <ul className="bookmark-list" aria-label="Bookmarks">
          {all.map((b) => (
            <Row key={b.id} b={b} />
          ))}
        </ul>
      )}
    </div>
  );
}
