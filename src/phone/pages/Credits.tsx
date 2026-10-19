/**
 * A person's credits on the phone: every record in the library they're
 * credited on, by role, from the files' tags and sleeve notes.
 */
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { plural } from "../../lib/format";
import { ROLE_PAGE_LABEL } from "../../lib/sleeve";
import type { CreditPage } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { nav } from "../../stores/router";
import { PageFailed } from "./common";
import { Empty, GhostRows, GoPill, PageFrame, Row } from "./kit";

type Entry = CreditPage["groups"][number]["albums"][number];

function songsLine(e: Entry): string {
  if (e.whole) return "Whole album";
  const titles = e.songs.map((s) => s.title);
  return titles.length <= 3 ? titles.join(", ") : `${titles.slice(0, 2).join(", ")} and ${titles.length - 2} more`;
}

export function PhoneCredits({ person }: { person: string }) {
  const { data, error, retry } = useLoad(() => backend.creditPage(person), [person]);
  const total = data ? new Set(data.groups.flatMap((g) => g.albums.map((e) => e.album.id))).size : 0;
  return (
    <PageFrame name="credits" title={data?.name ?? person} kicker="Credits" lede={data ? `Credited on ${plural(total, "album")} in your library.` : undefined}>
      {data?.artistId && (
        <div className="ph-x__pill-row">
          <GoPill route={{ name: "artist", id: data.artistId }}>Go to artist page</GoPill>
        </div>
      )}
      {error && data === undefined ? (
        <PageFailed what={`${person}'s credits`} retry={retry} />
      ) : data === undefined ? (
        <GhostRows />
      ) : data === null ? (
        <Empty title="No credits found" text={`Nothing in your library credits ${person} yet. Credits show up as albums' sleeve notes are opened and as your files' tags are read.`} />
      ) : (
        data.groups.map((g) => (
          <section key={g.role} className="ph-x__section" aria-label={ROLE_PAGE_LABEL[g.role] ?? g.role}>
            <h2 className="ph-x__day">
              <span>{ROLE_PAGE_LABEL[g.role] ?? g.role}</span>
              <span className="ph-x__count">{plural(g.albums.length, "album")}</span>
            </h2>
            <ul className="ph-rows">
              {g.albums.map((e) => (
                <Row
                  key={e.album.id}
                  art={<Art path={e.album.thumb} size={56} sleeve={{ title: e.album.title, artist: e.album.artistTitle }} />}
                  title={e.album.title}
                  sub={[e.album.artistTitle, e.album.year, songsLine(e)].filter(Boolean).join(" · ")}
                  onOpen={() => nav.go({ name: "album", id: e.album.id })}
                />
              ))}
            </ul>
          </section>
        ))
      )}
    </PageFrame>
  );
}
