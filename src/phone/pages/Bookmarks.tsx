/** Bookmarks on the phone: songs at a moment and pages you kept, newest first. */
import { X } from "lucide-react";
import { Art } from "../../components/Art";
import { formatDuration } from "../../lib/format";
import type { Bookmark } from "../../lib/types";
import { openBookmark, removeBookmark, useCurate } from "../../stores/curate";
import { Empty, PageFrame, Row } from "./kit";

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

/** "Song at 1:32" or "Album · Artist · Oct 2, 2026". */
export function bookmarkLine(b: Bookmark): string {
  if (b.kind === "track") return [b.subtitle, `at ${formatDuration(b.positionMs ?? 0)}`].filter(Boolean).join(" · ");
  return [PAGE_LABEL[b.page ?? ""] ?? "Page", b.subtitle, when(b.createdAt)].filter(Boolean).join(" · ");
}

export function PhoneBookmarks() {
  const all = useCurate((s) => s.bookmarks);
  return (
    <PageFrame name="bookmarks" title="Bookmarks">
      {all.length === 0 ? (
        <Empty title="No bookmarks yet" text="Bookmark a song at the moment you're at, or any album, artist, playlist or mix, from its menu." />
      ) : (
        <ul className="ph-rows" aria-label="Bookmarks">
          {all.map((b) => (
            <Row
              key={b.id}
              art={<Art path={b.thumb ?? null} size={48} round={b.page === "artist"} kind={b.page === "artist" ? "artist" : "album"} />}
              title={b.title}
              sub={bookmarkLine(b)}
              onOpen={() => void openBookmark(b)}
              end={
                <button type="button" className="ph-icon-btn" aria-label={`Remove ${b.title}`} onClick={() => void removeBookmark(b.id)}>
                  <X size={20} aria-hidden="true" />
                </button>
              }
            />
          ))}
        </ul>
      )}
    </PageFrame>
  );
}
