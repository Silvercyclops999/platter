/**
 * Built-in help: the pages in docs/help render in the app with no network,
 * search finds them (a few under All), "?" buttons open them, the docs site built
 * from the same Markdown has no broken links, and the writing follows the
 * house rules.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { HelpPanel } from "../help/HelpPanel";
import { HelpButton } from "../help/HelpButton";
import { WhatsNewDialog } from "../help/WhatsNewDialog";
import { HELP_ORDER } from "../help/order";
import { HELP_PAGES, helpImage, helpPage, helpSection, searchHelp } from "../help/pages";
import { helpLink, parse, slugify, toHtml } from "../help/markdown";
import { brokenLinks, buildSite } from "../help/site";
import { RELEASES, compareVersions, newSince } from "../help/whatsNew";
import { SearchPage } from "../pages/SearchPage";
import { help, useHelp } from "../stores/help";
import { loadSettings, updateSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
// The files as they are on disk, read the way the site build reads them.
const RAW = import.meta.glob("../../docs/help/*.md", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const IMAGES = Object.keys(import.meta.glob("../../docs/help/images/*"));
const sources = () => Object.entries(RAW).map(([path, md]) => ({ slug: path.replace(/^.*\/([^/]+)\.md$/, "$1"), md }));

beforeEach(async () => {
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useUi.getState().setHelpOpen(false);
  useHelp.setState({ slug: null, anchor: null });
});
afterEach(() => vi.restoreAllMocks());

describe("the help pages", () => {
  it("are all bundled, in reading order, with a title and a one-line summary", () => {
    expect(HELP_PAGES.map((p) => p.slug)).toEqual([...HELP_ORDER]);
    expect(sources().map((s) => s.slug).sort()).toEqual([...HELP_ORDER].sort());
    for (const p of HELP_PAGES) {
      expect(p.title, p.slug).not.toBe(p.slug);
      expect(p.summary.length, p.slug).toBeGreaterThan(20);
    }
  });

  it("render offline in the side panel, every one of them", async () => {
    // No call reaches the core while help shows (it's all bundled).
    const calls = Object.keys(backend).filter((k) => typeof (backend as unknown as Record<string, unknown>)[k] === "function");
    const spies = calls.map((k) => vi.spyOn(backend as never, k as never));
    render(<HelpPanel />);
    for (const p of HELP_PAGES) {
      act(() => help.open(p.slug));
      expect(await screen.findByRole("heading", { level: 2, name: p.title })).toBeInTheDocument();
      const panel = screen.getByRole("complementary", { name: "Help" });
      for (const b of p.blocks) {
        if (b.t === "h") expect(panel.querySelector(`[data-anchor="${b.id}"]`), `${p.slug}#${b.id}`).not.toBeNull();
        if (b.t === "img") expect(helpImage(b.src), `${p.slug} image ${b.src}`).toBeTruthy();
      }
    }
    for (const s of spies) expect(s).not.toHaveBeenCalled();
  });

  it("a link to another page opens it, an anchor goes to its section, outside links go through the core", async () => {
    const user = userEvent.setup();
    render(<HelpPanel />);
    act(() => help.open("lidarr", "the-safety-check"));
    const panel = await screen.findByRole("complementary", { name: "Help" });
    await waitFor(() => expect(document.activeElement?.getAttribute("data-anchor")).toBe("the-safety-check"));
    const toTroubleshooting = within(panel)
      .getAllByRole("link")
      .find((a) => a.getAttribute("href")?.startsWith("troubleshooting.md"));
    await user.click(toTroubleshooting!);
    expect(useHelp.getState().slug).toBe("troubleshooting");
    act(() => help.open("automation"));
    const out = within(screen.getByRole("complementary", { name: "Help" }))
      .getAllByRole("link")
      .find((a) => a.getAttribute("href")?.startsWith("https://"));
    mock.helpLinksOpened.length = 0;
    await user.click(out!);
    await waitFor(() => expect(mock.helpLinksOpened).toEqual(["https://github.com/Silvercyclops999/platter/blob/main/docs/AUTOMATION.md"]));
  });

  it("the list of pages and search inside the panel", async () => {
    const user = userEvent.setup();
    render(<HelpPanel />);
    act(() => help.open(""));
    const panel = await screen.findByRole("complementary", { name: "Help" });
    for (const p of HELP_PAGES) expect(within(panel).getByRole("button", { name: new RegExp(`^${p.title}`) })).toBeInTheDocument();
    await user.type(within(panel).getByRole("searchbox", { name: "Search help" }), "staging folder");
    await user.click(within(panel).getByRole("button", { name: /^Lidarr: The staging folder/ }));
    expect(useHelp.getState()).toMatchObject({ slug: "lidarr", anchor: "the-staging-folder" });
    await user.click(screen.getByRole("button", { name: "Close help" }));
    expect(useUi.getState().helpOpen).toBe(false);
  });
});

describe("search", () => {
  it("finds pages and sections, title words first", () => {
    expect(searchHelp("safety check")[0]).toMatchObject({ slug: "lidarr", anchor: "the-safety-check" });
    expect(searchHelp("graphics card").some((h) => h.slug === "lyrics-and-transcription")).toBe(true);
    expect(searchHelp("zzzz-nothing")).toEqual([]);
    expect(searchHelp("making your own")[0]).toMatchObject({ slug: "layouts", anchor: "making-your-own" });
    expect(searchHelp("page turn").some((h) => h.slug === "layouts")).toBe(true);
    expect(searchHelp("remember library filters").some((h) => h.slug === "your-library" && h.anchor === "filter-chips")).toBe(true);
    expect(searchHelp("printed sleeve").some((h) => h.slug === "your-library" && h.anchor === "albums-without-a-cover")).toBe(true);
    expect(searchHelp("same server").some((h) => h.slug === "connecting" && h.anchor === "jellyfin-and-subsonic-servers")).toBe(true);
  });

  it("the main search shows a few help results under All, with no Help chip", async () => {
    const user = userEvent.setup();
    render(<SearchPage query="practice mode" />);
    const results = await screen.findByRole("region", { name: "Help" });
    expect(within(results).getAllByRole("button").length).toBeLessThanOrEqual(3);
    expect(within(screen.getByRole("group", { name: "Filter results" })).queryByRole("button", { name: "Help" })).toBeNull();
    await user.click(within(results).getByRole("button", { name: /^Lidarr: Practice mode/ }));
    expect(useUi.getState().helpOpen).toBe(true);
    expect(useHelp.getState()).toMatchObject({ slug: "lidarr", anchor: "practice-mode" });
  });
});

describe("? buttons", () => {
  it("open the side panel, or the section in place inside a dialog, and hide when turned off", async () => {
    const user = userEvent.setup();
    const { rerender } = render(<HelpButton page="lidarr" anchor="the-safety-check" label="What the check's findings mean" />);
    await user.click(screen.getByRole("button", { name: "What the check's findings mean" }));
    expect(useHelp.getState()).toMatchObject({ slug: "lidarr", anchor: "the-safety-check" });
    rerender(<HelpButton page="tag-editor" anchor="check-tags" label="What Check tags looks for" inline />);
    const btn = screen.getByRole("button", { name: "What Check tags looks for" });
    await user.click(btn);
    expect(btn).toHaveAttribute("aria-expanded", "true");
    expect(await screen.findByRole("heading", { name: "Check tags" })).toBeInTheDocument();
    await act(() => updateSettings({ showHelpButtons: false }));
    expect(screen.queryByRole("button", { name: "What Check tags looks for" })).toBeNull();
  });

  it("a section is its heading up to the next one as big", () => {
    const blocks = helpSection("tag-editor", "check-tags");
    expect(blocks[0]).toMatchObject({ t: "h", id: "check-tags" });
    // Its ### subsections come along; the next ## doesn't.
    expect(blocks.some((b) => b.t === "h" && b.id === "one-click-fixes")).toBe(true);
    expect(blocks.some((b) => b.t === "h" && b.id === "plex-library-settings")).toBe(false);
  });
});

describe("the docs site", () => {
  it("builds every page and the contents, with no broken links or images", () => {
    const files = buildSite(sources());
    for (const slug of HELP_ORDER) expect(files.has(`${slug}.html`), slug).toBe(true);
    expect(files.has("index.html") && files.has("style.css")).toBe(true);
    const images = new Set(IMAGES.map((p) => p.replace(/^.*\/images\//, "images/")));
    expect(brokenLinks(files, images)).toEqual([]);
  });

  it("catches a broken link", () => {
    const files = buildSite([{ slug: "a", md: "# A\n\nSee [b](b.md) and [here](a.md#nowhere)." }]);
    expect(brokenLinks(files, new Set())).toEqual(["a.html: b.html", "a.html: a.html#nowhere"]);
  });

  it("outside links point only at the project, which the core lets help open", () => {
    for (const { slug, md } of sources()) {
      for (const [, href] of md.matchAll(/\]\(([^)\s]+)\)/g)) {
        if (helpLink(href) || href.startsWith("images/") || href.startsWith("#")) continue;
        expect(href, `${slug}: ${href}`).toMatch(/^https:\/\/github\.com\/Silvercyclops999\/platter\//);
      }
    }
  });
});

describe("the writing", () => {
  // The house rules for help text: no em dashes, and nothing from planning notes.
  const BANNED: [RegExp, string][] = [
    [/\u2014/, "an em dash"],
    [/\bspec\b/i, '"spec"'],
    [/\bphase\b/i, '"phase"'],
    [/CLAUDE\.md/, "CLAUDE.md"],
    [/\bREA\b/, '"REA"'],
  ];

  it("has no em dashes or planning references in docs/help", () => {
    for (const { slug, md } of sources()) {
      md.split("\n").forEach((line, i) => {
        for (const [re, what] of BANNED) expect(re.test(line), `${slug}.md:${i + 1} has ${what}: ${line}`).toBe(false);
      });
    }
  });

  it("the lint catches what it should", () => {
    const bad = ["Use it \u2014 like this", "See the spec", "In phase two", "Per CLAUDE.md", "REA said"];
    for (const line of bad) expect(BANNED.some(([re]) => re.test(line)), line).toBe(true);
    for (const line of ["A special setting", "Phased out? No.", "Read the area"]) expect(BANNED.some(([re]) => re.test(line)), line).toBe(false);
  });
});

describe("the Markdown reader", () => {
  it("reads headings, lists, links, code and comments the way the pages use them", () => {
    const blocks = parse("# Title\n\nOne *two* **three** `four` [five](x.md#y).\n<!-- TODO: hidden -->\n\n## Wrong lyrics?\n\n- a\n- b\n\n1. one\n2. two\n\n```\ncode here\n```\n");
    expect(blocks.map((b) => b.t)).toEqual(["h", "p", "h", "ul", "ol", "code"]);
    expect(blocks[2]).toMatchObject({ id: "wrong-lyrics" });
    const html = toHtml(blocks, { link: (h) => h, image: (s) => s });
    expect(html).toContain("<em>two</em>");
    expect(html).toContain("<strong>three</strong>");
    expect(html).toContain('<a href="x.md#y">five</a>');
    expect(html).not.toContain("TODO");
    expect(slugify("Music that isn't in your library")).toBe("music-that-isnt-in-your-library");
    expect(helpLink("lidarr.md#practice-mode")).toEqual({ slug: "lidarr", anchor: "practice-mode" });
    expect(helpLink("https://example.com/a.md")).toBeNull();
  });

  it("escapes text so a page can't add markup", () => {
    expect(toHtml(parse("a <b>c</b> & d"), { link: (h) => h, image: (s) => s })).toBe("<p>a &lt;b&gt;c&lt;/b&gt; &amp; d</p>");
  });
});

describe("What's new", () => {
  it("shows what came since the last version seen, once, with links to help", async () => {
    localStorage.setItem("platter.seenVersion.v1", "0.2.20");
    vi.spyOn(backend, "appInfo").mockResolvedValue({ version: "0.2.21", licence: "GPL-3.0-or-later", repoUrl: "" });
    const user = userEvent.setup();
    render(<WhatsNewDialog />);
    const d = await screen.findByRole("dialog", { name: "What's new in Platter" });
    expect(within(d).getAllByRole("listitem")).toHaveLength(RELEASES[0].items.length);
    await user.click(within(d).getAllByRole("button", { name: "Learn more" })[0]);
    expect(useUi.getState().helpOpen).toBe(true);
    expect(useHelp.getState()).toMatchObject({ slug: "getting-started", anchor: "getting-help" });
    expect(localStorage.getItem("platter.seenVersion.v1")).toBe("0.2.21");
  });

  it("nothing on a first run or the same version, and every link is real", () => {
    expect(newSince(null, "0.2.21")).toEqual([]);
    expect(newSince("0.2.21", "0.2.21")).toEqual([]);
    expect(newSince("0.2.19", "0.2.21").map((r) => r.version)).toEqual(["0.2.21"]);
    expect(compareVersions("0.2.10", "0.2.9")).toBe(1);
    for (const r of RELEASES)
      for (const it of r.items) {
        if (!it.help) continue;
        const p = helpPage(it.help[0]);
        expect(p, it.text).toBeTruthy();
        if (it.help[1]) expect(p!.blocks.some((b) => b.t === "h" && b.id === it.help![1])).toBe(true);
      }
  });
});
