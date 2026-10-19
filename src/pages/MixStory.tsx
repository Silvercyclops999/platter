/**
 * Under a mix's play row, on the computer and the phone: the mix in one
 * sentence (how many songs you know well and how many you've hardly
 * played, and who leads it), then its running order as a strip of thin
 * stems, tall for a song you know and short for one you don't, so you can
 * see where the new ones fall. The strip shows only when the mix holds both.
 */
import type { Track } from "../lib/types";
import "./ledger.css";

/** The two artists with the most songs in the mix, most first. */
export function leadArtists(tracks: Pick<Track, "artistTitle">[]): string[] {
  const n = new Map<string, number>();
  for (const t of tracks) if (t.artistTitle) n.set(t.artistTitle, (n.get(t.artistTitle) ?? 0) + 1);
  return [...n].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 2).map(([name]) => name);
}

export function mixSentence(tracks: Pick<Track, "artistTitle">[], familiar: boolean[]): string {
  const total = tracks.length;
  if (!total) return "";
  const known = familiar.slice(0, total).filter(Boolean).length;
  const fresh = total - known;
  const songs = (k: number) => `${k.toLocaleString()} ${k === 1 ? "song" : "songs"}`;
  const what =
    fresh === 0 ? `${songs(total)} you know well` : known === 0 ? `${songs(total)} you've hardly played` : `${songs(known)} you know well and ${fresh.toLocaleString()} you've hardly played`;
  const lead = leadArtists(tracks);
  const who = lead.length === 2 ? `, led by ${lead[0]} and ${lead[1]}` : lead.length === 1 ? `, all by ${lead[0]}` : "";
  return `${what}${who}.`;
}

/** What the strip says to a screen reader: "In running order: 1 song you know and 4 you've hardly played." */
export function stripLabel(known: number, fresh: number): string {
  return `In running order: ${known.toLocaleString()} ${known === 1 ? "song" : "songs"} you know and ${fresh.toLocaleString()} you've hardly played.`;
}

export function MixStory({ tracks, familiar, className = "" }: { tracks: Pick<Track, "id" | "artistTitle">[]; familiar: boolean[]; className?: string }) {
  if (!tracks.length) return null;
  const known = familiar.slice(0, tracks.length);
  const mixed = known.some(Boolean) && known.some((k) => !k);
  const k = known.filter(Boolean).length;
  return (
    <div className={`mix-story ${className}`} data-testid="mix-story">
      <p className="lg-lead lg-lead--xs mix-story__sentence">{mixSentence(tracks, familiar)}</p>
      {mixed && (
        <figure className="mix-strip" role="img" aria-label={stripLabel(k, tracks.length - k)}>
          <span className="mix-strip__songs" aria-hidden="true">
            {tracks.map((t, i) => (
              <span key={`${t.id}-${i}`} className={`mix-strip__song ${known[i] ? "is-known" : ""}`} />
            ))}
          </span>
          <figcaption className="mix-strip__key" aria-hidden="true">
            <span>First song</span>
            <span>Tall: songs you know. Short: ones you've hardly played.</span>
            <span>Last</span>
          </figcaption>
        </figure>
      )}
    </div>
  );
}
