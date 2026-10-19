import { useId, useMemo, useState } from "react";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import type { ArtistAlias, ArtistAutoPair, PlayedArtist } from "../lib/statsTypes";
import type { AppError } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./mixes.css";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/**
 * "Same artist as...": two names counted as one artist in Stats, Wrapped and
 * top lists. Each merge reads as a sentence with Undo, and a pair of fields
 * picks two played artists to join. Names Plex matched to one MusicBrainz
 * artist are joined on their own (while the row's switch is on) and listed
 * with "Not the same artist" to keep a pair apart.
 */
export function ArtistAliases({ ids }: { ids: { title: string; desc: string } }) {
  const auto = useSettings((s) => s.settings?.artistAliasesAuto !== false);
  const { data: loaded } = useLoad(() => backend.artistAliases(), []);
  const { data: played } = useLoad(() => backend.statsPlayedArtists("", 100), []);
  const { data: autoLoaded } = useLoad(() => backend.artistAliasAutoPairs(), [auto]);
  const [list, setList] = useState<ArtistAlias[] | null>(null);
  const [pairs, setPairs] = useState<ArtistAutoPair[] | null>(null);
  const all = list ?? loaded ?? [];
  const autoPairs = auto ? (pairs ?? autoLoaded ?? []) : [];
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const listId = useId();
  const byName = useMemo(() => new Map((played ?? []).map((a: PlayedArtist) => [a.name.toLowerCase(), a])), [played]);
  /** A played artist by the name typed: the picker's first, then the whole history. */
  const pick = async (t: string): Promise<{ key: string; name: string } | null> => {
    const s = t.trim();
    if (!s) return null;
    const known = byName.get(s.toLowerCase());
    if (known) return known;
    try {
      const found = (await backend.statsPlayedArtists(s, 20)).find((a) => a.name.toLowerCase() === s.toLowerCase());
      return found ?? null;
    } catch {
      return null;
    }
  };
  const fail = (e: unknown) => toast.error(errText(e) || "Couldn't save that.");
  const set = async (a: { key: string; name?: string; sameAs?: string; sameAsName?: string }, on: boolean) => {
    try {
      setList(await backend.artistAliasSet(a, on));
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  };
  const apart = async (p: { key: string; name: string; other: string; otherName: string }, on: boolean) => {
    try {
      setList(await backend.artistAliasApart(p, on));
      setPairs(await backend.artistAliasAutoPairs());
    } catch (e) {
      fail(e);
    }
  };
  const add = async () => {
    const [a, b] = await Promise.all([pick(from), pick(to)]);
    const missing = !a ? from : !b ? to : null;
    if (missing !== null) {
      toast.error(`No artist you've played is called ${missing.trim()}.`);
      return;
    }
    if (a && b && (await set({ key: a.key, name: a.name, sameAs: b.key, sameAsName: b.name }, true))) {
      setFrom("");
      setTo("");
    }
  };
  const joined = all.filter((a) => !a.apart);
  const keptApart = all.filter((a) => a.apart);
  return (
    <div className="artist-aliases">
      {joined.length > 0 && (
        <ul className="artist-aliases__list" aria-label="Artists counted as one">
          {joined.map((a) => (
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
      {autoPairs.length > 0 && (
        <ul className="artist-aliases__list" aria-label="Matched to one artist">
          {autoPairs.map((p) => (
            <li key={`${p.key}|${p.sameAs}`} className="artist-aliases__row">
              <span className="artist-aliases__text">
                <em>{p.name}</em> and <em>{p.sameAsName}</em> are matched to one artist
              </span>
              <Button
                size="sm"
                onClick={() => void apart({ key: p.key, name: p.name, other: p.sameAs, otherName: p.sameAsName }, true)}
                aria-label={`${p.name} and ${p.sameAsName} are not the same artist`}
              >
                Not the same artist
              </Button>
            </li>
          ))}
        </ul>
      )}
      {keptApart.length > 0 && (
        <ul className="artist-aliases__list" aria-label="Kept apart">
          {keptApart.map((a) => (
            <li key={`${a.key}|${a.sameAs}`} className="artist-aliases__row">
              <span className="artist-aliases__text">
                <em>{a.name}</em> and <em>{a.sameAsName}</em> count apart
              </span>
              <Button
                size="sm"
                onClick={() => void apart({ key: a.key, name: a.name, other: a.sameAs, otherName: a.sameAsName }, false)}
                aria-label={`Count ${a.name} and ${a.sameAsName} as one again`}
              >
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
