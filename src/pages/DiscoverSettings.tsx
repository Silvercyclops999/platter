/** Settings: the artists marked "Not interested", each with a way back. */
import { Button } from "../components/Button";
import { undismissArtist, useDismissed } from "./catalogue";
import "./catalogue.css";

export function DismissedArtists() {
  const list = useDismissed();
  if (list.length === 0) return <p className="setting-row__note">None yet. "Not interested" on an artist you don't know adds them here.</p>;
  return (
    <ul className="dismissed-list" aria-label="Artists you're not interested in">
      {[...list].reverse().map((d) => (
        <li key={d.mbid} className="dismissed-list__item">
          <span className="dismissed-list__name">{d.name || "Unnamed artist"}</span>
          <Button size="sm" variant="ghost" onClick={() => undismissArtist(d.mbid)} aria-label={`Show ${d.name} in suggestions again`}>
            Undo
          </Button>
        </li>
      ))}
    </ul>
  );
}
