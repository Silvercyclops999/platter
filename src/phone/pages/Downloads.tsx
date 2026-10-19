/**
 * Downloads on the phone: what's on the phone (each album, playlist, Liked
 * Songs or mix with where it stands, from its songs), the storage it uses,
 * Go offline, and Remove all. The same download store and core commands as
 * the computer.
 */
import { Heart, ListMusic, Disc3, Radio } from "lucide-react";
import { useState } from "react";
import { Dialog } from "../../components/Dialog";
import { suggestionLine, useDownloadSuggestions } from "../../components/DownloadSuggestions";
import { Switch } from "../../components/Switch";
import { formatBytes } from "../../lib/format";
import type { DownloadItem } from "../../lib/types";
import { pauseText, removeAllDownloads, retryDownload, useDownloads } from "../../stores/downloads";
import { nav, type Route } from "../../stores/router";
import { updateSettings, useSettings } from "../../stores/settings";
import { PhoneDownload, countText, phoneDownloadState } from "../PhoneDownload";
import { Empty, GhostRows, GoPill, PageFrame, Row } from "./kit";

const KIND_WORD: Record<DownloadItem["kind"], string> = { album: "Album", playlist: "Playlist", liked: "Playlist", mix: "Mix" };
const KIND_ICON = { album: Disc3, playlist: ListMusic, liked: Heart, mix: Radio } as const;

function routeOf(i: DownloadItem): Route {
  if (i.kind === "album") return { name: "album", id: i.id };
  if (i.kind === "playlist") return { name: "playlist", id: i.id };
  if (i.kind === "liked") return { name: "liked" };
  return i.id.startsWith("topSongs:") ? (i.id === "topSongs:all" ? { name: "topSongs" } : { name: "topSongs", year: Number(i.id.slice(9)) }) : { name: "mix", id: i.id };
}

/** "Downloaded · 19 songs", "Downloading · 12/19", "3 songs didn't download". */
export function downloadLine(i: DownloadItem): string {
  const state = phoneDownloadState(i);
  const kind = KIND_WORD[i.kind];
  if (state === "done") return `${kind} · ${i.total === 1 ? "1 song" : `${i.total} songs`} · ${formatBytes(i.bytes)}`;
  if (state === "error") return `${kind} · ${i.failed + i.missing} of ${i.total} didn't download`;
  if (state === "downloading") return `${kind} · Downloading ${countText(i)}`;
  return `${kind} · Waiting${i.total ? ` · ${countText(i)}` : ""}`;
}

export function PhoneDownloads() {
  const status = useDownloads((s) => s.status);
  const offline = useSettings((s) => !!s.settings?.goOffline);
  const [asking, setAsking] = useState(false);
  const suggested = useDownloadSuggestions();
  const items = status?.items ?? [];
  const share = status && status.limitBytes > 0 ? Math.min(1, status.usedBytes / status.limitBytes) : 0;
  const waiting = status?.queued ? pauseText(status.paused) : null;
  return (
    <PageFrame name="downloads" title="Downloads" lede="Music kept on this phone, to play without a connection.">
      <section className="ph-card ph-dl-card" aria-label="Storage">
        <p className="ph-dl-card__used tabular">
          {status ? `${formatBytes(status.usedBytes)} of ${formatBytes(status.limitBytes)} used` : " "}
          {status?.freeBytes != null ? `, ${formatBytes(status.freeBytes)} free on this phone` : ""}
        </p>
        <span className="ph-dl-card__meter" role="meter" aria-label="Download storage used" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(share * 100)}>
          <span style={{ width: `${share * 100}%` }} />
        </span>
        {waiting && <p className="ph-x__note">{`${status!.queued === 1 ? "1 song" : `${status!.queued} songs`} to download. ${waiting}`}</p>}
        <div className="ph-dl-card__offline">
          <span className="ph-row__text">
            <span className="ph-row__title" id="ph-dl-offline">
              Go offline
            </span>
            <span className="ph-row__sub">Only downloaded music plays; nothing else is fetched.</span>
          </span>
          <Switch checked={offline} onChange={(v) => void updateSettings({ goOffline: v })} labelledBy="ph-dl-offline" />
        </div>
      </section>
      {!status ? (
        <GhostRows />
      ) : items.length === 0 ? (
        <Empty title="Nothing downloaded yet" text="Tap the download mark on an album, playlist, Liked Songs or a mix to keep it on this phone." />
      ) : (
        <ul className="ph-rows" aria-label="On this phone">
          {items.map((i) => {
            const Icon = KIND_ICON[i.kind];
            const r = routeOf(i);
            return (
              <Row
                key={`${i.kind}:${i.id}`}
                label={i.title}
                art={
                  <span className="ph-row__glyph" aria-hidden="true">
                    <Icon size={22} />
                  </span>
                }
                title={i.title}
                sub={downloadLine(i)}
                onOpen={() => nav.go(r)}
                end={
                  phoneDownloadState(i) === "error" ? (
                    <button type="button" className="ph-pill" onClick={() => void retryDownload(i.kind, i.id)}>
                      Try again
                    </button>
                  ) : (
                    <PhoneDownload kind={i.kind} id={i.id} title={i.title} />
                  )
                }
              />
            );
          })}
        </ul>
      )}
      {suggested.list.length > 0 && (
        <section aria-labelledby="ph-dl-other">
          <h2 id="ph-dl-other" className="ph-label">
            On your other devices
          </h2>
          <ul className="ph-rows" aria-label="On your other devices">
            {suggested.list.map((x) => {
              const Icon = KIND_ICON[x.kind];
              return (
                <Row
                  key={`${x.kind}:${x.id}`}
                  label={x.title}
                  art={
                    <span className="ph-row__glyph" aria-hidden="true">
                      <Icon size={22} />
                    </span>
                  }
                  title={x.title}
                  sub={suggestionLine(x)}
                  end={
                    <span className="ph-x__pill-row">
                      <button type="button" className="ph-pill" onClick={() => suggested.dismiss(x)}>
                        Not now
                      </button>
                      <button type="button" className="ph-pill is-primary" onClick={() => void suggested.download(x)}>
                        Download here
                      </button>
                    </span>
                  }
                />
              );
            })}
          </ul>
        </section>
      )}
      <div className="ph-x__pill-row">
        {items.length > 0 && (
          <button type="button" className="ph-pill" onClick={() => setAsking(true)}>
            Remove all downloads
          </button>
        )}
        <GoPill route={{ name: "settings", section: "Downloads and devices" }}>Download settings</GoPill>
      </div>
      <Dialog open={asking} onClose={() => setAsking(false)} label="Remove all downloads?" className="confirm">
        <h2 className="confirm__title">Remove all downloads?</h2>
        <p className="confirm__body">You won't be able to play them offline. Liked Songs stops downloading too.</p>
        <div className="confirm__actions">
          <button type="button" className="ph-pill" onClick={() => setAsking(false)}>
            Cancel
          </button>
          <button
            type="button"
            className="ph-pill is-primary"
            onClick={() => {
              setAsking(false);
              void removeAllDownloads();
            }}
          >
            Remove
          </button>
        </div>
      </Dialog>
    </PageFrame>
  );
}
