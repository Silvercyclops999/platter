/**
 * Instrument tokens for Platter's own looks: the engraved plates (format,
 * bit-perfect, counts, the layouts' plates) and the dark glass of the now
 * playing readouts. instruments.css carries the same values; a test checks
 * the two match and that every text colour clears WCAG AA on its plate or
 * glass at its size.
 *
 * Plates and glass are opaque, so their text contrast doesn't depend on
 * what they sit on. Their edges are decoration: the text is what identifies
 * them.
 */
export type InstrumentLook = "studio" | "studioBlack" | "sleeve";

export interface PlateTokens {
  /** The plate's face. */
  bg: string;
  /** Plain plate text (formats, counts, speeds). */
  text: string;
  /** Lit plate text: brass, for hi-res and bit-perfect. */
  lit: string;
  /** Plate text when something fell back (bit-perfect playing shared). */
  warn: string;
  /** The hairline round the plate. */
  edge: string;
}

export interface GlassTokens {
  /** The glass, top and bottom of its slight gradient. */
  top: string;
  bottom: string;
  /** Readout text, a quieter line, and the lit (brass) line. */
  text: string;
  dim: string;
  lit: string;
}

export const PLATES: Record<InstrumentLook, PlateTokens> = {
  studio: { bg: "#2D2A26", text: "#D3CABC", lit: "#D9BB7B", warn: "#F2B13D", edge: "#4D4842" },
  studioBlack: { bg: "#1C1A18", text: "#CFC6B8", lit: "#D4B575", warn: "#F2B13D", edge: "#3E3A35" },
  sleeve: { bg: "#FAF6EC", text: "#3F372D", lit: "#6A4C14", warn: "#7A4E00", edge: "#B9AC92" },
};

/** Dark glass on every look, Sleeve's paper included: a readout is a lit window. */
export const GLASS: Record<InstrumentLook, GlassTokens> = {
  studio: { top: "#0E0D0B", bottom: "#161411", text: "#EFE6D6", dim: "#B9AE9C", lit: "#DDBF80" },
  studioBlack: { top: "#050504", bottom: "#0C0B0A", text: "#EFE6D6", dim: "#B3A896", lit: "#DDBF80" },
  sleeve: { top: "#1E1A15", bottom: "#27221C", text: "#F3EADB", dim: "#C4B8A4", lit: "#E2C68C" },
};

/** Plate text is 11px (small text): it needs the full 4.5:1. */
export const PLATE_MIN_CONTRAST = 4.5;
