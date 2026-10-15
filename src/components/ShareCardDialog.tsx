/**
 * Share › Share card…: a preview of the card in either format, a few
 * choices, and Copy image / Save image. The card is drawn here on a canvas;
 * the core puts it on the clipboard or in the file the user picks.
 */
import { Copy, Download } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { backend } from "../lib/backend";
import { headerColorsFromImageUrl, NEUTRAL_HEADER } from "../lib/color";
import { trackArtistLine } from "../lib/format";
import { canvasMeasure, CARD_SIZES, drawCard, layoutCard, type CardContent, type CardFormat } from "../lib/shareCard";
import type { CardQr, PublicLinkState } from "../lib/types";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useShareCard, type CardSubject } from "../stores/shareCard";
import { toast } from "../stores/toasts";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { Switch } from "./Switch";
import "./share-card.css";

/** Said when the canvas can't give its pixels. */
export const CARD_FAILED = "Couldn't make the image";

/** Title, subtitle and cover for a subject, looked up when not given. */
async function describe(s: CardSubject): Promise<{ title: string; subtitle: string; thumb: string | null; durationMs: number }> {
  if (s.title != null && s.subtitle != null)
    return { title: s.title, subtitle: s.subtitle, thumb: s.thumb ?? null, durationMs: s.durationMs ?? 0 };
  if (s.kind === "album") {
    const p = await backend.albumPage(s.id);
    return { title: p.album.title, subtitle: p.album.artistTitle, thumb: p.album.thumb, durationMs: p.totalDurationMs };
  }
  if (s.kind === "playlist") {
    const p = await backend.playlistPage(s.id);
    const n = p.tracks.length;
    return { title: p.playlist.title, subtitle: `Playlist · ${n} ${n === 1 ? "song" : "songs"}`, thumb: p.playlist.thumb, durationMs: 0 };
  }
  const t = usePlayer.getState().state?.track;
  if (t?.id === s.id) return { title: t.title, subtitle: trackArtistLine(t), thumb: t.thumb, durationMs: t.durationMs };
  return { title: s.title ?? "", subtitle: s.subtitle ?? "", thumb: s.thumb ?? null, durationMs: s.durationMs ?? 0 };
}

/** Where the song is now, when it's the one playing or paused. */
function playingAtMs(s: CardSubject): number | null {
  if (s.kind !== "track") return null;
  const { state, positionMs } = usePlayer.getState();
  if (state?.track?.id !== s.id || (state.status !== "playing" && state.status !== "paused")) return null;
  return positionMs;
}

function loadImage(url: string | undefined): Promise<HTMLImageElement | null> {
  if (!url || typeof Image === "undefined") return Promise.resolve(null);
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => resolve(img);
    img.onerror = () => resolve(null);
    img.src = url;
  });
}

function ShareCard({ subject, onClose }: { subject: CardSubject; onClose: () => void }) {
  const settings = useSettings((s) => s.settings);
  const [format, setFormat] = useState<CardFormat>("story");
  const [atMs] = useState(() => playingAtMs(subject));
  const lyricCard = !!subject.lyrics?.length;
  const [info, setInfo] = useState<Awaited<ReturnType<typeof describe>> | null>(null);
  const [art, setArt] = useState<HTMLImageElement | null>(null);
  const [vivid, setVivid] = useState<string>(NEUTRAL_HEADER.vivid);
  const [publicState, setPublicState] = useState<PublicLinkState | null>(null);
  const [songLink, setSongLink] = useState<string | null>(null);
  const [qr, setQr] = useState<CardQr | null>(null);
  const [showProgress, setShowProgress] = useState(true);
  const [showQr, setShowQr] = useState(settings?.shareCardQr ?? true);
  const [showLink, setShowLink] = useState(settings?.shareCardSongLink ?? true);
  const [busy, setBusy] = useState(false);
  const canvas = useRef<HTMLCanvasElement>(null);
  const atSeconds = atMs != null && showProgress && !lyricCard ? Math.floor(atMs / 1000) : undefined;

  useEffect(() => {
    let live = true;
    describe(subject).then(
      (d) => {
        if (!live) return;
        setInfo(d);
        if (d.thumb) void loadImage(backend.artUrl(d.thumb, 800, 800)).then((img) => live && setArt(img));
        if (d.thumb) void headerColorsFromImageUrl(backend.artUrl(d.thumb, 64, 64)).then((c) => live && c && setVivid(c.vivid));
      },
      () => live && setInfo({ title: subject.title ?? "", subtitle: subject.subtitle ?? "", thumb: null, durationMs: 0 }),
    );
    backend.shareInfo(subject.kind, subject.id).then(
      (i) => live && setPublicState(i.public),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [subject]);

  // The song.link line: released music only, when public links and the line are on.
  const linkAllowed = publicState === "available" && !lyricCard;
  useEffect(() => {
    if (!linkAllowed || !showLink) return;
    let live = true;
    backend.sharePublicLink(subject.kind, subject.id).then(
      (u) => live && setSongLink(u),
      () => live && setSongLink(null),
    );
    return () => {
      live = false;
    };
  }, [linkAllowed, showLink, subject]);

  useEffect(() => {
    if (!showQr) return;
    let live = true;
    backend.shareCardQr(subject.kind, subject.id, atSeconds).then(
      (q) => live && setQr(q),
      () => live && setQr(null),
    );
    return () => {
      live = false;
    };
  }, [showQr, subject, atSeconds]);

  const content: CardContent | null = useMemo(() => {
    if (!info) return null;
    return {
      title: info.title,
      subtitle: info.subtitle,
      thumb: info.thumb,
      lyrics: subject.lyrics ?? null,
      progress: atSeconds != null && info.durationMs > 0 ? { atMs: atMs!, durationMs: info.durationMs } : null,
      songLink: linkAllowed && showLink ? songLink : null,
      qr: showQr && qr ? { size: qr.size, rows: qr.rows } : null,
    };
  }, [info, subject.lyrics, atSeconds, atMs, linkAllowed, showLink, songLink, showQr, qr]);

  useEffect(() => {
    const el = canvas.current;
    if (!el || !content) return;
    const { width, height } = CARD_SIZES[format];
    el.width = width;
    el.height = height;
    let ctx: CanvasRenderingContext2D | null = null;
    try {
      ctx = el.getContext("2d");
    } catch {
      ctx = null;
    }
    if (!ctx) return;
    drawCard(ctx, layoutCard(format, content, canvasMeasure(ctx)), content, art, { vivid });
  }, [content, format, art, vivid]);

  const ready = !!content;
  const name = `${info?.title ?? "Platter"}${lyricCard ? " lyrics" : ""}`;

  const copy = async () => {
    const el = canvas.current;
    const ctx = el?.getContext("2d");
    if (!el || !ctx) return void toast.error(CARD_FAILED);
    setBusy(true);
    try {
      const data = ctx.getImageData(0, 0, el.width, el.height).data;
      await backend.shareCardCopy(new Uint8Array(data.buffer, data.byteOffset, data.byteLength), el.width, el.height);
      toast.info("Image copied");
    } catch {
      toast.error("Couldn't copy the image");
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    const el = canvas.current;
    if (!el) return;
    setBusy(true);
    try {
      const blob = await new Promise<Blob | null>((resolve) => el.toBlob(resolve, "image/png"));
      if (!blob) throw new Error("no image");
      const saved = await backend.shareCardSave(new Uint8Array(await blob.arrayBuffer()), name);
      if (saved) toast.info(`Saved ${saved}`);
    } catch {
      toast.error("Couldn't save the image");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="sharecard">
      <h2 className="sharecard__title">{lyricCard ? "Share lyrics" : "Share card"}</h2>
      <div className="sharecard__formats" role="radiogroup" aria-label="Card shape">
        {(["story", "square"] as const).map((f) => (
          <button
            key={f}
            type="button"
            role="radio"
            aria-checked={format === f}
            className={`sharecard__format ${format === f ? "is-on" : ""}`}
            onClick={() => setFormat(f)}
          >
            {f === "story" ? "Story" : "Square"}
          </button>
        ))}
      </div>
      <div className={`sharecard__preview sharecard__preview--${format}`}>
        <canvas ref={canvas} className="sharecard__canvas" role="img" aria-label={`${format === "story" ? "Story" : "Square"} card for ${info?.title ?? "this item"}`} data-format={format} />
      </div>
      <div className="sharecard__options">
        {atMs != null && !lyricCard && (
          <label className="sharecard__option">
            <span id="sc-progress">Show where the song is</span>
            <Switch checked={showProgress} onChange={setShowProgress} labelledBy="sc-progress" />
          </label>
        )}
        <label className="sharecard__option">
          <span id="sc-qr">QR code that opens it in Platter</span>
          <Switch checked={showQr} onChange={setShowQr} labelledBy="sc-qr" />
        </label>
        {linkAllowed && (
          <label className="sharecard__option">
            <span id="sc-link">song.link line</span>
            <Switch checked={showLink} onChange={setShowLink} labelledBy="sc-link" />
          </label>
        )}
        <p className="sharecard__note">
          {publicState === "unreleased"
            ? "Unreleased music never gets a song.link line. The QR code works for people on your server."
            : "The QR code works for people on your server. Nothing is uploaded."}
        </p>
      </div>
      <div className="sharecard__actions">
        <Button variant="ghost" onClick={onClose}>
          Close
        </Button>
        <Button icon={<Download size={16} />} disabled={!ready || busy} onClick={() => void save()}>
          Save image
        </Button>
        <Button variant="primary" icon={<Copy size={16} />} disabled={!ready || busy} onClick={() => void copy()}>
          Copy image
        </Button>
      </div>
    </div>
  );
}

/** The share card dialog, opened through `openShareCard`. */
export function ShareCardHost() {
  const subject = useShareCard((s) => s.subject);
  const close = useShareCard((s) => s.close);
  return (
    <Dialog open={!!subject} onClose={close} label={subject?.lyrics?.length ? "Share lyrics" : "Share card"} className="sharecard-dialog">
      {subject && <ShareCard key={`${subject.kind}:${subject.id}:${subject.lyrics?.join("|") ?? ""}`} subject={subject} onClose={close} />}
    </Dialog>
  );
}
