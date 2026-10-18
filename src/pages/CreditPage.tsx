/**
 * A person's credit page: everything in the library they're credited on,
 * grouped by role ("Produced", "Played on"), from the files' tags and the
 * sleeve notes of albums opened so far. Opens from any name in sleeve notes
 * or the Credits dialog.
 */
import { Fragment } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { LoadError } from "../components/LoadError";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { plural } from "../lib/format";
import { ROLE_PAGE_LABEL } from "../lib/sleeve";
import type { CreditPage as CreditPageData } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { nav } from "../stores/router";
import { SourceMarks } from "./SleeveNotes";
import "./sleeve.css";

type Group = CreditPageData["groups"][number];
type Entry = Group["albums"][number];

function songsLine(e: Entry): string {
  if (e.whole) return "Whole album";
  const titles = e.songs.map((s) => s.title);
  return titles.length <= 3 ? titles.join(", ") : `${titles.slice(0, 2).join(", ")} and ${titles.length - 2} more`;
}

function EntryRow({ e }: { e: Entry }) {
  const a = e.album;
  const sub = [a.artistTitle, a.year, e.details.join(", ")].filter(Boolean).join(" · ");
  return (
    <li className="credit-row">
      <button type="button" className="credit-row__open" onClick={() => nav.go({ name: "album", id: a.id })}>
        <Art path={a.thumb} size={56} alt="" />
        <span className="credit-row__text">
          <span className="credit-row__title truncate">{a.title}</span>
          <span className="credit-row__sub truncate">{sub}</span>
          <span className="credit-row__songs truncate">{songsLine(e)}</span>
        </span>
      </button>
      <SourceMarks sources={e.sources} />
    </li>
  );
}

export function CreditPage({ person }: { person: string }) {
  const { data, error, retry } = useLoad(() => backend.creditPage(person), [person]);
  if (error && data === undefined) return <LoadError what={`${person}'s credits`} error={error} onRetry={retry} />;
  const total = data ? new Set(data.groups.flatMap((g) => g.albums.map((e) => e.album.id))).size : 0;
  return (
    <div className="page-pad credit-page">
      <header className="credit-page__head">
        <span className="credit-page__kicker">Credits</span>
        <h1 className="credit-page__name">{data?.name ?? person}</h1>
        {data === undefined ? (
          <Skeleton width={220} height={14} />
        ) : data ? (
          <p className="credit-page__lede">
            Credited on {plural(total, "album")} in your library.
            {data.artistId && (
              <Button size="sm" variant="ghost" onClick={() => nav.go({ name: "artist", id: data.artistId! })}>
                Go to artist page
              </Button>
            )}
          </p>
        ) : null}
      </header>
      {data === undefined ? (
        <div aria-busy="true">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} width="50%" height={56} style={{ margin: "8px 0" }} />
          ))}
        </div>
      ) : data === null ? (
        <div className="empty-state" role="status">
          <h2 className="empty-state__title">No credits found</h2>
          <p className="empty-state__text">
            Nothing in your library credits {person} yet. Credits show up as albums' sleeve notes are opened and as your files' tags are read.
          </p>
        </div>
      ) : (
        data.groups.map((g) => (
          <Fragment key={g.role}>
            <section className="credit-group" aria-labelledby={`credit-${g.role}`}>
              <h2 id={`credit-${g.role}`} className="credit-group__title">
                {ROLE_PAGE_LABEL[g.role] ?? g.role}
                <span className="credit-group__count">{plural(g.albums.length, "album")}</span>
              </h2>
              <ol className="credit-rows">
                {g.albums.map((e) => (
                  <EntryRow key={e.album.id} e={e} />
                ))}
              </ol>
            </section>
          </Fragment>
        ))
      )}
    </div>
  );
}
