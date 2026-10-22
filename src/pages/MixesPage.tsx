/**
 * Radio and mixes: every mix Platter made for you, by kind, on one page.
 * Each kind is a ruled list: the mix's picture, its name, who is in it,
 * its length and Play.
 * Radio starts from any song, album or artist menu, so the page says so.
 * New mixes show as soon as they are made; offline, only downloaded ones.
 */
import { Pause, Play } from "lucide-react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { LoadError } from "../components/LoadError";
import { Skeleton } from "../components/Skeleton";
import { duration } from "../lib/statsStory";
import type { MixSummary } from "../lib/types";
import { useApp } from "../stores/app";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useContextState } from "./cards";
import { openMix } from "./MixCard";
import { useMixShelves } from "./mixShelves";
import "./home.css";
import "./ledger.css";
import "./mixes.css";
import "../shell/facets.css";

/**
 * A mix's picture in the list, where the row prints its name: the leading
 * artist's photo, else four of its covers, else plain paper. (A printed
 * sleeve's type can't be read this small.) The daylist shows its covers.
 */
export function MixSleeve({ mix, size }: { mix: MixSummary; size: number }) {
  const photo = mix.kind === "daylist" || mix.kind === "topSongs" ? null : mix.art;
  if (photo) return <Art path={photo} fluid requestSize={size * 2} kind="album" />;
  if (!mix.covers.length) return <Art path={null} fluid kind="album" />;
  return (
    <span className="mix-art" aria-hidden="true">
      <span className="mix-art__collage">
        {Array.from({ length: 4 }, (_, i) => (
          <Art key={i} path={mix.covers[i % mix.covers.length]} fluid requestSize={size} kind="album" />
        ))}
      </span>
    </span>
  );
}

function MixRow({ mix }: { mix: MixSummary }) {
  const { isContext, playing } = useContextState(mix.kind === "radio" ? "radio" : "mix", mix.id);
  const title = mix.kind === "daylist" ? "daylist" : mix.title;
  const about = mix.kind === "daylist" ? "Your day in a playlist." : mix.description;
  return (
    // The whole row opens the mix (the title is its keyboard stop); Play keeps its own click.
    <li
      className={`mix-row ${playing ? "is-playing" : ""}`}
      data-testid={`mix-card-${mix.id}`}
      onClick={(e) => {
        if (!(e.target as HTMLElement).closest("button")) openMix(mix);
      }}
    >
      <span className="mix-row__sleeve">
        <MixSleeve mix={mix} size={64} />
      </span>
      <span className="mix-row__text">
        <button type="button" className="mix-row__title" onClick={() => openMix(mix)}>
          {title}
        </button>
        {about && <span className="mix-row__about">{about}</span>}
      </span>
      {mix.trackCount > 0 && (
        <span className="mix-row__length">
          {mix.trackCount.toLocaleString()} {mix.trackCount === 1 ? "song" : "songs"}
          {mix.durationMs > 0 && <small>{duration(mix.durationMs)}</small>}
        </span>
      )}
      <IconButton
        label={`${playing ? "Pause" : "Play"} ${title}`}
        size="md"
        variant={playing ? "accent" : "filled"}
        icon={playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="nudge-right" />}
        onClick={() => (isContext ? player.toggle() : void player.playMix(mix.id))}
      />
    </li>
  );
}

function MixList({ title, mixes }: { title: string; mixes: MixSummary[] }) {
  if (!mixes.length) return null;
  return (
    <section className="mixes-page__group" aria-label={title}>
      <div className="lg-head">
        <h2 className="lg-head__title">{title}</h2>
        <span className="lg-head__aside">{mixes.length}</span>
      </div>
      <ul className="mix-rows" aria-label={title}>
        {mixes.map((m) => (
          <MixRow key={m.id} mix={m} />
        ))}
      </ul>
    </section>
  );
}

export function MixesPage() {
  const on = useSettings((s) => s.settings?.homeMixes !== false);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const { data, error, retry } = useMixShelves(on);

  const count = data ? data.madeForYou.length + data.moods.length + data.genres.length : 0;
  return (
    <div className="page-pad mixes-page">
      <h1 className="page-title">Radio and mixes</h1>
      <p className="mixes-page__lede">Start radio from any song, album or artist with its menu. Mixes refresh as you listen.</p>
      {!on ? (
        <div className="empty-state" role="status">
          <h2 className="empty-state__title">Mixes are off</h2>
          <p className="empty-state__text">Turn on mixes in Settings and they show here.</p>
          <button type="button" className="facet" onClick={() => nav.go({ name: "settings", row: "homeMixes" })}>
            Open Settings
          </button>
        </div>
      ) : error && !data ? (
        <LoadError what="Your mixes" error={error} onRetry={retry} inline />
      ) : !data ? (
        <ul className="mix-rows" aria-busy="true" aria-label="Loading mixes">
          {Array.from({ length: 6 }, (_, i) => (
            <li key={i} className="mix-row" aria-hidden="true">
              <Skeleton width={64} height={64} radius="row" />
              <span className="mix-row__text">
                <Skeleton width="40%" height={16} />
                <Skeleton width="60%" height={12} style={{ marginTop: 6 }} />
              </span>
            </li>
          ))}
        </ul>
      ) : count === 0 && cachedOnly ? (
        <div className="empty-state" role="status">
          <h2 className="empty-state__title">No mixes on this computer</h2>
          <p className="empty-state__text">Download a mix from its page and it plays here when Platter can't reach your server.</p>
        </div>
      ) : count === 0 ? (
        <div className="empty-state" role="status">
          <h2 className="empty-state__title">No mixes yet</h2>
          <p className="empty-state__text">Play a few albums and your first mixes appear here within a day.</p>
        </div>
      ) : (
        <>
          <MixList title="Made for you" mixes={data.madeForYou} />
          <MixList title="By mood" mixes={data.moods} />
          <MixList title="By genre" mixes={data.genres} />
        </>
      )}
    </div>
  );
}
