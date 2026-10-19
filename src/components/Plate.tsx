/**
 * An engraved plate: the one small label for formats, bit-perfect, counts
 * and the layouts' plates. Plain text in the plate's ink, lit (brass) only
 * for hi-res and bit-perfect, warn when something fell back. Its colours are
 * the look's plate tokens (instruments.css); Classic draws a plain outline.
 */
import type { ButtonHTMLAttributes, ReactNode } from "react";

export type PlateTone = "plain" | "lit" | "warn";

interface PlateProps {
  tone?: PlateTone;
  /** A small lamp before the text, lit with the plate's tone when `lampOn`. */
  lamp?: boolean;
  lampOn?: boolean;
  /** Small: the plate inside a list row or a header's line. */
  size?: "sm" | "md";
  className?: string;
  title?: string;
  children: ReactNode;
}

function classes(p: Pick<PlateProps, "tone" | "size" | "className">, extra = ""): string {
  return ["plate", p.size === "sm" ? "plate--sm" : "", p.tone === "lit" ? "is-lit" : p.tone === "warn" ? "is-warn" : "", extra, p.className ?? ""]
    .filter(Boolean)
    .join(" ");
}

function Lamp({ on }: { on?: boolean }) {
  return <span className={`plate__lamp ${on ? "is-on" : ""}`} aria-hidden="true" />;
}

export function Plate({ tone = "plain", lamp, lampOn, size = "md", className, title, children }: PlateProps) {
  return (
    <span className={classes({ tone, size, className })} title={title}>
      {lamp && <Lamp on={lampOn} />}
      {children}
    </span>
  );
}

/** A plate that is also a button (the signal path's). */
export function PlateButton({
  tone = "plain",
  size = "md",
  className,
  children,
  ...rest
}: Omit<PlateProps, "lamp" | "lampOn"> & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button type="button" {...rest} className={classes({ tone, size, className }, "plate--button")}>
      {children}
    </button>
  );
}
