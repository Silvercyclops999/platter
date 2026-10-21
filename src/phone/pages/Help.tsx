/**
 * Help on the phone, as a page of its own: the list of help pages with a
 * search, or one page. The same bundled pages as the computer's help panel,
 * so it works offline, less the pages about things only the computer has.
 */
import { ChevronRight } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { HelpView } from "../../help/HelpView";
import { COMPUTER_ONLY_HELP } from "../../help/order";
import { HELP_PAGES, helpPage, searchHelp } from "../../help/pages";
import "../../help/help.css";
import { nav } from "../../stores/router";
import { PageFrame } from "./kit";

export function PhoneHelp({ slug, anchor }: { slug?: string; anchor?: string }) {
  const page = slug ? helpPage(slug) : undefined;
  const [query, setQuery] = useState("");
  const body = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!anchor) return;
    body.current?.querySelector<HTMLElement>(`[data-anchor="${CSS.escape(anchor)}"]`)?.scrollIntoView?.({ block: "start" });
  }, [slug, anchor]);

  if (page)
    return (
      <PageFrame name="help" title={page.title} lede={page.summary || undefined}>
        <div className="ph-help-page" ref={body}>
          <HelpView slug={page.slug} blocks={page.blocks.filter((b, i) => !(i === 0 && b.t === "p"))} />
        </div>
      </PageFrame>
    );

  const onPhone = (slug: string) => !(COMPUTER_ONLY_HELP as readonly string[]).includes(slug);
  const hits = query.trim() ? searchHelp(query).filter((h) => onPhone(h.slug)) : null;
  const list = hits ?? HELP_PAGES.filter((p) => onPhone(p.slug)).map((p) => ({ slug: p.slug, title: p.title, snippet: p.summary, anchor: undefined as string | undefined, page: undefined as string | undefined }));
  return (
    <PageFrame name="help" title="Help" lede="Short pages on every part of Platter. They work offline.">
      <label className="ph-field ph-set-search">
        <span className="sr-only">Search help</span>
        <input
          type="search"
          className="ph-field__input"
          value={query}
          placeholder="Search help"
          enterKeyHint="search"
          autoCorrect="off"
          autoCapitalize="none"
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>
      <ul className="ph-set-list" aria-label="Help pages">
        {list.map((h) => (
          <li key={`${h.slug}#${h.anchor ?? ""}`}>
            <button type="button" className="ph-set-link" onClick={() => nav.go({ name: "help", slug: h.slug, ...(h.anchor ? { anchor: h.anchor } : {}) })}>
              <span className="ph-set-link__text">
                <span className="ph-set-link__title">{h.page ? `${h.page}: ${h.title}` : h.title}</span>
                {h.snippet && <span className="ph-set-link__sub">{h.snippet}</span>}
              </span>
              <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
            </button>
          </li>
        ))}
      </ul>
      {hits && hits.length === 0 && <p className="ph-rows__note">No help matches "{query.trim()}".</p>}
    </PageFrame>
  );
}
