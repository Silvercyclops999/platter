import { ArrowLeft, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { IconButton } from "../components/Button";
import { SearchField } from "../components/SearchField";
import { help, useHelp } from "../stores/help";
import { useUi } from "../stores/ui";
import { shortcutLabel } from "../shell/shortcuts";
import { HelpView } from "./HelpView";
import { HELP_PAGES, helpPage, searchHelp } from "./pages";
import "./help.css";

/** Help in the right slot, next to what it explains. Bundled, so it works offline. */
export function HelpPanel() {
  const slug = useHelp((s) => s.slug);
  const anchor = useHelp((s) => s.anchor);
  const tick = useHelp((s) => s.tick);
  const body = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const page = slug ? helpPage(slug) : undefined;

  useEffect(() => {
    const el = body.current;
    if (!el) return;
    const target = anchor ? el.querySelector<HTMLElement>(`[data-anchor="${CSS.escape(anchor)}"]`) : null;
    if (target) {
      target.scrollIntoView?.({ block: "start" });
      target.focus({ preventScroll: true });
    } else {
      el.scrollTop = 0;
    }
  }, [slug, anchor, tick]);

  const hits = query.trim() ? searchHelp(query) : [];
  return (
    <aside className="queue-panel help-panel" aria-label="Help">
      <header className="queue-panel__head">
        {page && (
          <IconButton label="All help pages" icon={<ArrowLeft size={18} />} onClick={() => useHelp.setState({ slug: null, anchor: null })} />
        )}
        <h2 className="help-panel__title truncate">{page ? page.title : "Help"}</h2>
        <IconButton label="Close help" icon={<X size={18} />} onClick={help.close} />
      </header>
      <div className="queue-panel__body help-panel__body" ref={body}>
        {page ? (
          <>
            {page.summary && <p className="help-panel__lede">{page.summary}</p>}
            <HelpView slug={page.slug} blocks={page.blocks.filter((b, i) => !(i === 0 && b.t === "p"))} />
          </>
        ) : (
          <>
            <div className="help-panel__search">
              <SearchField value={query} onChange={setQuery} label="Search help" placeholder="Search help" />
            </div>
            {!query.trim() && (
              <button type="button" className="help-panel__item help-panel__keys" onClick={() => useUi.getState().setShortcutsOpen(true)}>
                <span className="help-panel__item-title">Keyboard shortcuts</span>
                <span className="help-panel__item-snippet">Every key Platter knows. Also {shortcutLabel("help")}.</span>
              </button>
            )}
            <ul className="help-panel__list">
              {(query.trim() ? hits : HELP_PAGES.map((p) => ({ slug: p.slug, title: p.title, snippet: p.summary, anchor: undefined, page: undefined }))).map(
                (h) => (
                  <li key={`${h.slug}#${h.anchor ?? ""}`}>
                    <button type="button" className="help-panel__item" onClick={() => help.open(h.slug, h.anchor)}>
                      <span className="help-panel__item-title">{h.page ? `${h.page}: ${h.title}` : h.title}</span>
                      {h.snippet && <span className="help-panel__item-snippet">{h.snippet}</span>}
                    </button>
                  </li>
                ),
              )}
            </ul>
            {query.trim() && hits.length === 0 && <p className="help-panel__empty">No help matches "{query.trim()}".</p>}
          </>
        )}
      </div>
    </aside>
  );
}
