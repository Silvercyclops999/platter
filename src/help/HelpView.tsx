import { Fragment, type ReactNode } from "react";
import { backend } from "../lib/backend";
import { help } from "../stores/help";
import { helpLink, type Block, type Inline } from "./markdown";
import { helpImage } from "./pages";

function follow(href: string) {
  const h = helpLink(href);
  if (h) help.open(h.slug, h.anchor);
  else if (href.startsWith("#")) help.open(currentSlug(), href.slice(1));
  else void backend.helpOpenLink(href).catch(() => {});
}

let shownSlug = "";
const currentSlug = () => shownSlug;

function inline(c: Inline[]): ReactNode {
  return c.map((n, i) => {
    switch (n.t) {
      case "text":
        return <Fragment key={i}>{n.v}</Fragment>;
      case "code":
        return <code key={i}>{n.v}</code>;
      case "strong":
        return <strong key={i}>{inline(n.c)}</strong>;
      case "em":
        return <em key={i}>{inline(n.c)}</em>;
      case "img": {
        const url = helpImage(n.src);
        return url ? <img key={i} src={url} alt={n.alt} /> : null;
      }
      case "link":
        return (
          <a
            key={i}
            href={n.href}
            onClick={(e) => {
              e.preventDefault();
              follow(n.href);
            }}
          >
            {inline(n.c)}
          </a>
        );
    }
  });
}

/** A help page's body, as React (the docs site gets the same blocks as HTML). */
export function HelpView({ slug, blocks }: { slug: string; blocks: Block[] }) {
  shownSlug = slug;
  return (
    <div className="help-view">
      {blocks.map((b, i) => {
        switch (b.t) {
          case "h": {
            const H = b.level === 1 ? "h2" : b.level === 2 ? "h3" : "h4";
            return (
              <H key={i} id={`help-${b.id}`} data-anchor={b.id} tabIndex={-1}>
                {inline(b.c)}
              </H>
            );
          }
          case "p":
            return <p key={i}>{inline(b.c)}</p>;
          case "ul":
            return (
              <ul key={i}>
                {b.items.map((it, j) => (
                  <li key={j}>{inline(it)}</li>
                ))}
              </ul>
            );
          case "ol":
            return (
              <ol key={i}>
                {b.items.map((it, j) => (
                  <li key={j}>{inline(it)}</li>
                ))}
              </ol>
            );
          case "code":
            return (
              <pre key={i}>
                <code>{b.v}</code>
              </pre>
            );
          case "img": {
            const url = helpImage(b.src);
            return url ? <img key={i} className="help-view__img" src={url} alt={b.alt} /> : null;
          }
        }
      })}
    </div>
  );
}
