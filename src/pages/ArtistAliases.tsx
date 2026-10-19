import { useId, useMemo, useState } from "react";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import type { ArtistAlias, PlayedArtist } from "../lib/statsTypes";
import type { AppError } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { toast } from "../stores/toasts";
import "./mixes.css";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/**
 * "Same artist as...": two names counted as one artist in Stats, Wrapped and
 * top lists. Each merge reads as a sentence with Undo, and a pair of fields
 * picks two played artists to join. Artists matched to one MusicBrainz id
 * are joined on their own and aren't listed.
 */
export function ArtistAliases({ ids }: { ids: { title: string; desc: string } }) {
  const { data: loaded } = useLoad(() => backend.artistAliases(), []);
  const { data: played } = useLoad(() => backend.statsPlayedArtists("", 100), []);
  const [list, setList] = useState<ArtistAlias[] | null>(null);
  const all = list ?? loaded ?? [];
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const listId = useId();
  const byName = useMemo(() => new Map((played ?? []).map((a: PlayedArtist) => [a.name.toLowerCase(), a])), [played]);
  const pick = (t: string): { key: string; name: string } | null => {
    const s = t.trim();
    if (!s) return null;
    return byName.get(s.toLowerCase()) ?? { key: s.toLowerCase(), name: s };
  };
  const set = async (a: { key: string; name?: string; sameAs?: string; sameAsName?: string }, on: boolean) => {
    try {
      setList(await backend.artistAliasSet(a, on));
      return true;
    } catch (e) {
      toast.error(errText(e) || "Couldn't save that.");
      return false;
    }
  };
  const add = async () => {
    const a = pick(from);
    const b = pick(to);
    if (!a || !b) return;
    if (await set({ key: a.key, name: a.name, sameAs: b.key, sameAsName: b.name }, true)) {
      setFrom("");
      setTo("");
    }
  };
  return (
    <div className="artist-aliases">
      {all.length > 0 && (
        <ul className="artist-aliases__list" aria-label="Artists counted as one">
          {all.map((a) => (
            <li key={a.key} className="artist-aliases__row">
              <span className="artist-aliases__text">
                <em>{a.name}</em> counts as <em>{a.sameAsName}</em>
              </span>
              <Button size="sm" onClick={() => void set({ key: a.key }, false)} aria-label={`Count ${a.name} on its own again`}>
                Undo
              </Button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="mix-exclusions__add artist-aliases__add"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <input
          className="setting-text"
          list={listId}
          value={from}
          placeholder="An artist"
          aria-label="Artist"
          aria-describedby={ids.desc}
          onChange={(e) => setFrom(e.target.value)}
        />
        <span className="artist-aliases__join">is the same artist as</span>
        <input
          className="setting-text"
          list={listId}
          value={to}
          placeholder="Another name"
          aria-label="Same artist as"
          onChange={(e) => setTo(e.target.value)}
        />
        <datalist id={listId}>
          {(played ?? []).map((a) => (
            <option key={a.key} value={a.name} />
          ))}
        </datalist>
        <Button variant="secondary" size="sm" type="submit" disabled={!from.trim() || !to.trim()}>
          Count as one
        </Button>
      </form>
    </div>
  );
}
