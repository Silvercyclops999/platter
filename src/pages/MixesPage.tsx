/**
 * Radio and mixes: every mix Platter made for you, by kind, on one page.
 * Radio starts from any song, album or artist menu, so the page says so.
 */
import { useEffect, useState } from "react";
import { CardSkeleton } from "../components/Card";
import { LoadError } from "../components/LoadError";
import { backend } from "../lib/backend";
import type { MixShelves, MixSummary } from "../lib/types";
import { useApp } from "../stores/app";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { MixCard } from "./MixCard";
import "./home.css";
import "../shell/facets.css";

function MixGrid({ title, mixes }: { title: string; mixes: MixSummary[] }) {
  if (!mixes.length) return null;
  return (
    <section className="mixes-page__group" aria-label={title}>
      <h2 className="section-title">{title}</h2>
      <div className="shelf-page__grid" role="list" aria-label={title}>
        {mixes.map((m) => (
          <div key={m.id} role="listitem">
            <MixCard mix={m} />
          </div>
        ))}
      </div>
    </section>
  );
}

export function MixesPage() {
  const on = useSettings((s) => s.settings?.homeMixes !== false);
  const libraryVersion = useApp((s) => s.libraryVersion);
  const [data, setData] = useState<MixShelves | null>(null);
  const [error, setError] = useState<{ message: string } | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!on) return;
    let alive = true;
    backend.mixesHome().then(
      (m) => alive && (setData(m), setError(null)),
      (e: { message?: string }) => alive && setError({ message: e?.message ?? "Couldn't load your mixes." }),
    );
    return () => {
      alive = false;
    };
  }, [on, libraryVersion, attempt]);

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
        <LoadError what="Your mixes" error={error} onRetry={() => setAttempt((n) => n + 1)} inline />
      ) : !data ? (
        <div className="shelf-page__grid" aria-busy="true" aria-label="Loading mixes">
          {Array.from({ length: 8 }, (_, i) => (
            <div key={i} aria-hidden="true">
              <CardSkeleton kind="album" />
            </div>
          ))}
        </div>
      ) : count === 0 ? (
        <div className="empty-state" role="status">
          <h2 className="empty-state__title">No mixes yet</h2>
          <p className="empty-state__text">Play a few albums and your first mixes appear here within a day.</p>
        </div>
      ) : (
        <>
          <MixGrid title="Made for you" mixes={data.madeForYou} />
          <MixGrid title="By mood" mixes={data.moods} />
          <MixGrid title="By genre" mixes={data.genres} />
        </>
      )}
    </div>
  );
}
