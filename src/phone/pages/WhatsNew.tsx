/** What's new on the phone: releases added to the library by artists you follow, New then Earlier. */
import { useEffect, useRef, useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { addedLabel } from "../../lib/relativeTime";
import type { NewRelease, WhatsNew } from "../../lib/types";
import { releaseType } from "../../pages/WhatsNewPage";
import { useApp } from "../../stores/app";
import { phone } from "../phoneUi";
import { MoreHorizontal } from "lucide-react";
import { nav } from "../../stores/router";
import { PageFailed } from "./common";
import { Empty, GhostRows, PageFrame, Row } from "./kit";

function Group({ title, items }: { title: string; items: NewRelease[] }) {
  if (!items.length) return null;
  return (
    <section className="ph-x__section" aria-label={title}>
      <h2 className="ph-section-title">{title}</h2>
      <ul className="ph-rows">
        {items.map(({ album: a }) => (
          <Row
            key={a.id}
            art={<Art path={a.thumb} size={64} sleeve={{ title: a.title, artist: a.artistTitle }} />}
            title={a.title}
            sub={`${a.artistTitle} · ${releaseType(a)} · ${addedLabel(a.addedAt)}`}
            onOpen={() => nav.go({ name: "album", id: a.id })}
            end={
              <button type="button" className="ph-icon-btn" aria-label={`More for ${a.title}`} onClick={() => phone.push({ kind: "album", album: a })}>
                <MoreHorizontal size={20} aria-hidden="true" />
              </button>
            }
          />
        ))}
      </ul>
    </section>
  );
}

export function PhoneWhatsNew() {
  const [data, setData] = useState<WhatsNew | null>(null);
  const [error, setError] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const libraryVersion = useApp((s) => s.libraryVersion);
  // What was new while the page is open stays in New.
  const newIds = useRef(new Set<string>());
  useEffect(() => {
    let alive = true;
    setError(false);
    backend.whatsNew().then(
      (w) => {
        if (!alive) return;
        for (const i of w.items) if (i.new) newIds.current.add(i.album.id);
        setData({ ...w, items: w.items.map((i) => (newIds.current.has(i.album.id) ? { ...i, new: true } : i)) });
        if (w.unseen > 0) void backend.whatsNewSeen().catch(() => {});
      },
      () => alive && setError(true),
    );
    return () => {
      alive = false;
    };
  }, [attempt, libraryVersion]);
  return (
    <PageFrame
      name="whatsNew"
      title="What's new"
      lede={data && !data.following ? "New in your library. Follow artists to see only their releases here." : "The latest releases from artists you follow."}
    >
      {error && !data ? (
        <PageFailed what="What's new" retry={() => setAttempt((n) => n + 1)} />
      ) : !data ? (
        <GhostRows n={4} />
      ) : data.items.length === 0 ? (
        <Empty title="Nothing new yet" text="When an artist you follow gets new music in your library, it shows up here." />
      ) : (
        <>
          <Group title="New" items={data.items.filter((i) => i.new)} />
          <Group title="Earlier" items={data.items.filter((i) => !i.new)} />
        </>
      )}
    </PageFrame>
  );
}
