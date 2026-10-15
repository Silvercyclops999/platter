/**
 * Lines under lyrics lines: a romanised pronunciation (made by the core on
 * this computer, while Show pronunciation is on) and a translation (from
 * the service set in Settings, after Translate is pressed).
 */
import { Languages } from "lucide-react";
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { AppError } from "../lib/types";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";

export interface Extras {
  pron: (string | null)[] | null;
  trans: string[] | null;
  /** A translation service is set. */
  canTranslate: boolean;
  translating: boolean;
  toggleTranslation: () => void;
}

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

export function useLyricExtras(texts: string[]): Extras {
  const show = useSettings((s) => !!s.settings?.showPronunciation);
  const service = useSettings((s) => s.settings?.translationService ?? "");
  const target = useSettings((s) => s.settings?.translateTo ?? "en");
  const [pron, setPron] = useState<(string | null)[] | null>(null);
  const [trans, setTrans] = useState<string[] | null>(null);
  const [on, setOn] = useState(false);
  const [translating, setTranslating] = useState(false);
  const key = texts.join("\n");

  useEffect(() => {
    setPron(null);
    if (!show || !texts.some((t) => t.trim())) return;
    let live = true;
    backend
      .lyricsPronounce(texts)
      .then((p) => live && setPron(p.some((x) => x) ? p : null))
      .catch(() => {
        /* no pronunciation: the lyrics show as they are */
      });
    return () => {
      live = false;
    };
  }, [show, key]);

  // New lyrics, another language or no service: the translation goes.
  useEffect(() => {
    setTrans(null);
    setOn(false);
  }, [key, service, target]);

  useEffect(() => {
    if (!on || trans || !service) return;
    let live = true;
    setTranslating(true);
    backend
      .lyricsTranslate(texts)
      .then((t) => live && setTrans(t))
      .catch((e) => {
        if (!live) return;
        setOn(false);
        toast.error(errText(e) || "Couldn't translate these lyrics");
      })
      .finally(() => live && setTranslating(false));
    return () => {
      live = false;
    };
  }, [on, trans, service, key]);

  return {
    pron,
    trans: on ? trans : null,
    canTranslate: !!service,
    translating,
    toggleTranslation: () => setOn((v) => !v),
  };
}

/** The pronunciation and translation for line `i`, if any. */
export function LyricExtras({ extras, i }: { extras: Extras; i: number }) {
  const p = extras.pron?.[i];
  const t = extras.trans?.[i];
  if (!p && !t) return null;
  return (
    <>
      {p && (
        <span className="lyrics__extra lyrics__extra--pron" lang="en-Latn">
          {p}
        </span>
      )}
      {t && <span className="lyrics__extra lyrics__extra--trans">{t}</span>}
    </>
  );
}

/** Translate / Hide translation, under the lyrics, when a service is set. */
export function TranslateButton({ extras }: { extras: Extras }) {
  if (!extras.canTranslate) return null;
  const shown = !!extras.trans;
  return (
    <p className="lyrics__share">
      <button type="button" className="lyrics__source-edit" onClick={extras.toggleTranslation} disabled={extras.translating} aria-pressed={shown}>
        <Languages size={14} aria-hidden="true" />
        {extras.translating ? "Translating…" : shown ? "Hide translation" : "Translate"}
      </button>
    </p>
  );
}
