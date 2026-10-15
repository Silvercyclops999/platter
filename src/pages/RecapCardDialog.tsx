/**
 * Share a month's recap as an image: the same Copy image / Save image as
 * share cards, drawn here. Unreleased songs never go on the card.
 */
import { Copy, Download } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { CARD_FAILED, loadImage } from "../components/ShareCardDialog";
import { backend } from "../lib/backend";
import { headerColorsFromImageUrl } from "../lib/color";
import { drawRecapCard, layoutRecapCard, recapContent } from "../lib/recapCard";
import { canvasMeasure, CARD_SIZES, type CardFormat } from "../lib/shareCard";
import { monthName } from "../lib/stats";
import type { MonthRecap } from "../lib/statsTypes";
import { toast } from "../stores/toasts";
import "../components/share-card.css";

export function RecapCardDialog({ recap, onClose }: { recap: MonthRecap; onClose: () => void }) {
  const [format, setFormat] = useState<CardFormat>("story");
  const [busy, setBusy] = useState(false);
  const canvas = useRef<HTMLCanvasElement>(null);
  const base = useMemo(() => recapContent(recap), [recap]);
  const [covers, setCovers] = useState<(HTMLImageElement | null)[]>([]);
  const [tint, setTint] = useState<string | null>(null);
  const content = useMemo(() => (tint ? { ...base, color: tint } : base), [base, tint]);
  const name = monthName(recap.month);

  // The month's covers, and the card's colour from the first of them.
  useEffect(() => {
    let live = true;
    void Promise.all(base.covers.map((c) => loadImage(backend.artUrl(c, 320, 320)))).then((imgs) => live && setCovers(imgs));
    if (base.covers[0]) void headerColorsFromImageUrl(backend.artUrl(base.covers[0], 64, 64)).then((c) => live && c && setTint(c.vivid));
    return () => {
      live = false;
    };
  }, [base]);

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
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
    drawRecapCard(ctx, layoutRecapCard(format, content, canvasMeasure(ctx)), content, covers);
  }, [content, format, covers]);

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
      const saved = await backend.shareCardSave(new Uint8Array(await blob.arrayBuffer()), `${name} recap`);
      if (saved) toast.info(`Saved ${saved}`);
    } catch {
      toast.error("Couldn't save the image");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open onClose={onClose} label={`Share ${name}`} className="sharecard-dialog">
      <div className="sharecard" data-testid="recap-share">
        <h2 className="sharecard__title">Share {name}</h2>
        <div className="sharecard__formats" role="radiogroup" aria-label="Card shape">
          {(["story", "square"] as const).map((f) => (
            <button key={f} type="button" role="radio" aria-checked={format === f} className={`sharecard__format ${format === f ? "is-on" : ""}`} onClick={() => setFormat(f)}>
              {f === "story" ? "Story" : "Square"}
            </button>
          ))}
        </div>
        <div className={`sharecard__preview sharecard__preview--${format}`}>
          <canvas ref={canvas} className="sharecard__canvas" role="img" aria-label={`${format === "story" ? "Story" : "Square"} card for ${name}`} data-format={format} />
        </div>
        <p className="sharecard__note">Unreleased songs are left off the card. Nothing is uploaded.</p>
        <div className="sharecard__actions">
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
          <Button icon={<Download size={16} />} disabled={busy} onClick={() => void save()}>
            Save image
          </Button>
          <Button variant="primary" icon={<Copy size={16} />} disabled={busy} onClick={() => void copy()}>
            Copy image
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
