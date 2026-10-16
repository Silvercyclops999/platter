/**
 * Share cards: the layout of each kind of card, the dialog (opening it from
 * Share, the formats, copy and save), the QR code holding only the Platter
 * link, unreleased music never getting a song.link line, and picking lyric
 * lines for a lyric card.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost, openContextMenu } from "../components/ContextMenu";
import { ShareCardHost } from "../components/ShareCardDialog";
import { shareItem } from "../components/ShareMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyricsKind } from "../lib/mock/details";
import { mockHasNoMatch } from "../lib/mock/share";
import { CARD_SIZES, layoutCard, wrapText, type CardContent, type Measure } from "../lib/shareCard";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { clearLyricsCache } from "../shell/lyrics";
import { LyricsPane } from "../shell/LyricsView";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { openShareCard, useLyricPick, useShareCard } from "../stores/shareCard";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const fx = mock.fx;
const demoAlbum = fx.albums.find((a) => a.subformats.includes("demo"))!;
const releasedAlbums = fx.albums.filter((a) => !a.subformats.includes("demo") && a.format !== "demo");
const released = fx.tracks.find((t) => releasedAlbums.some((a) => a.id === t.albumId) && !mockHasNoMatch(t.id))!;
const releasedAlbum = releasedAlbums.find((a) => !mockHasNoMatch(a.id))!;
const demoTrack = fx.tracks.find((t) => t.albumId === demoAlbum.id)!;

/** A pretend text measure: every character is 0.55 of the font size. */
const measure: Measure = (t, size) => t.length * size * 0.55;

const base: CardContent = { title: "Signal Fire", subtitle: "Copperline Choir", thumb: "/thumb/1" };
const qr = { size: 21, rows: Array.from({ length: 21 }, () => "1".repeat(21)) };

/** A canvas that draws nothing but can hand over pixels and a PNG. */
function fakeCanvas() {
  const ctx = new Proxy(
    {
      measureText: (t: string) => ({ width: t.length * 10 }),
      createLinearGradient: () => ({ addColorStop: () => {} }),
      getImageData: (_x: number, _y: number, w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }),
    } as Record<string, unknown>,
    { get: (o, k: string) => (k in o ? o[k] : () => {}), set: () => true },
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
  vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation((cb: BlobCallback) =>
    cb(new Blob([new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3])], { type: "image/png" })),
  );
}

function playing(track: Track, positionMs: number) {
  const s: PlayerState = {
    status: "paused",
    track,
    positionMs,
    durationMs: track.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: null, userQueue: [], nextUp: [], history: [], context: null },
  };
  act(() => usePlayer.setState({ state: s, positionMs, durationMs: track.durationMs }));
}

beforeEach(async () => {
  useToasts.setState({ toasts: [] });
  await backend.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  mock.cardsCopied = [];
  mock.cardsSaved = [];
  mock.cardSaveCancels = false;
  fakeCanvas();
});

afterEach(() => {
  // Close first, so nothing still open draws on a canvas that's real again.
  act(() => useShareCard.getState().close());
  act(() => useUi.setState({ menu: null }));
  useLyricPick.getState().cancel();
  usePlayer.setState({ state: null, local: null, positionMs: 0 });
  vi.restoreAllMocks();
});

const toasts = () => useToasts.getState().toasts.map((t) => t.message);

describe("card layout", () => {
  it("a story card: the cover, title, artist, progress, link, QR and mark", () => {
    const l = layoutCard("story", { ...base, progress: { atMs: 61_000, durationMs: 183_000 }, songLink: "https://song.link/i/1", qr }, measure);
    expect([l.width, l.height]).toEqual([1080, 1920]);
    expect(l.art).toEqual({ x: 140, y: 260, size: 800, radius: 20 });
    expect(l.texts.title.lines).toEqual(["Signal Fire"]);
    expect(l.progress).toMatchObject({ frac: 61_000 / 183_000, left: "1:01", right: "3:03" });
    expect(l.texts.link?.lines).toEqual(["https://song.link/i/1"]);
    expect(l.qr).toEqual({ x: 800, y: 1640, size: 200 });
    // Nothing runs into the footer.
    expect(l.texts.link!.y).toBeLessThan(l.qr!.y);
    expect(l).toMatchSnapshot();
  });

  it("a square card, with no extras", () => {
    const l = layoutCard("square", base, measure);
    expect([l.width, l.height]).toEqual([CARD_SIZES.square.width, CARD_SIZES.square.height]);
    expect(l.art?.size).toBe(560);
    expect([l.progress, l.qr, l.texts.link, l.texts.lyrics]).toEqual([null, null, null, null]);
    expect(l.texts.subtitle.y).toBeLessThan(l.mark.y);
    expect(l).toMatchSnapshot();
  });

  it("a lyric card puts the picked lines where the cover was", () => {
    const l = layoutCard("story", { ...base, lyrics: ["One", "", "Two", "Three", "Four", "Five"], qr }, measure);
    expect(l.art?.size).toBe(140);
    expect(l.texts.lyrics?.lines).toEqual(["One", "Two", "Three", "Four"]);
    expect(l.texts.link).toBeNull();
    expect(l).toMatchSnapshot();
  });

  it("long titles wrap to two lines and end in an ellipsis", () => {
    const lines = wrapText("a very long song title that keeps going on and on and on forever", 400, 40, 800, measure, 2);
    expect(lines).toHaveLength(2);
    expect(lines[1].endsWith("…")).toBe(true);
    expect(lines.every((x) => measure(x, 40, 800) <= 400)).toBe(true);
  });
});

describe("the share card dialog", () => {
  it("opens from Share › Share card… with the album's title", async () => {
    const user = userEvent.setup();
    render(
      <>
        <ContextMenuHost />
        <ShareCardHost />
      </>,
    );
    act(() => openContextMenu("Options", [shareItem("album", releasedAlbum.id, releasedAlbum.thumb)], { clientX: 10, clientY: 10 }));
    await user.click(await screen.findByRole("menuitem", { name: "Share" }));
    await user.click(await screen.findByRole("menuitem", { name: "Share card…" }));
    const dialog = await screen.findByRole("dialog", { name: "Share card" });
    await waitFor(() => expect(within(dialog).getByRole("img", { name: `Story card for ${releasedAlbum.title}` })).toBeInTheDocument());
  });

  it("artists get no share card", async () => {
    const user = userEvent.setup();
    render(<ContextMenuHost />);
    act(() => openContextMenu("Options", [shareItem("artist", fx.artists[0].id)], { clientX: 10, clientY: 10 }));
    await user.click(await screen.findByRole("menuitem", { name: "Share" }));
    const sub = await screen.findByRole("menu", { name: "Share" });
    expect(within(sub).queryByRole("menuitem", { name: "Share card…" })).toBeNull();
  });

  it("copies the story card, then the square one", async () => {
    render(<ShareCardHost />);
    act(() => openShareCard({ kind: "album", id: releasedAlbum.id }));
    const copy = await screen.findByRole("button", { name: "Copy image" });
    await waitFor(() => expect(copy).toBeEnabled());
    fireEvent.click(copy);
    await waitFor(() => expect(mock.cardsCopied).toEqual([{ width: 1080, height: 1920, bytes: 1080 * 1920 * 4 }]));
    // The toast and the button coming back follow the copy's answer, a step later.
    await waitFor(() => expect(toasts()).toContain("Image copied"));
    await waitFor(() => expect(copy).toBeEnabled());
    fireEvent.click(screen.getByRole("radio", { name: "Square" }));
    expect(screen.getByRole("radio", { name: "Square" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(copy);
    await waitFor(() => expect(mock.cardsCopied[1]).toMatchObject({ width: 1080, height: 1080 }));
  });

  it("saves a PNG through the save dialog, and a cancelled dialog says nothing", async () => {
    render(<ShareCardHost />);
    act(() => openShareCard({ kind: "album", id: releasedAlbum.id }));
    const save = await screen.findByRole("button", { name: "Save image" });
    await waitFor(() => expect(save).toBeEnabled());
    fireEvent.click(save);
    await waitFor(() => expect(mock.cardsSaved).toHaveLength(1));
    expect(mock.cardsSaved[0].name).toMatch(/\.png$/);
    await waitFor(() => expect(toasts()).toContain(`Saved ${mock.cardsSaved[0].name}`));
    await waitFor(() => expect(save).toBeEnabled());
    useToasts.setState({ toasts: [] });
    mock.cardSaveCancels = true;
    fireEvent.click(save);
    expect(save).toBeDisabled();
    await waitFor(() => expect(save).toBeEnabled());
    expect(mock.cardsSaved).toHaveLength(1);
    expect(toasts()).toEqual([]);
  });

  it("a canvas that can't give its pixels says so", async () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    render(<ShareCardHost />);
    act(() => openShareCard({ kind: "album", id: releasedAlbum.id }));
    const copy = await screen.findByRole("button", { name: "Copy image" });
    await waitFor(() => expect(copy).toBeEnabled());
    fireEvent.click(copy);
    await waitFor(() => expect(toasts()).toContain("Couldn't make the image"));
  });

  it("a song playing now can show where it is, and its QR holds only the Platter link", async () => {
    const qrSpy = vi.spyOn(backend, "shareCardQr");
    playing(released, 83_000);
    render(<ShareCardHost />);
    act(() => openShareCard({ kind: "track", id: released.id, title: released.title, subtitle: "x", durationMs: released.durationMs }));
    expect(await screen.findByRole("switch", { name: "Show where the song is" })).toHaveAttribute("aria-checked", "true");
    await waitFor(() => expect(qrSpy).toHaveBeenCalledWith("track", released.id, 83));
    // The answer to that call, whichever call it was.
    const at = qrSpy.mock.calls.findIndex((c) => c[2] === 83);
    const q = await qrSpy.mock.results[at].value;
    expect(q.text).toMatch(new RegExp(`^platter://song/${released.id}\\?t=83&s=[0-9a-f]{8}$`));
    expect(q.text).not.toMatch(/http|plex|mock-machine|\d+\.\d+\.\d+\.\d+/);
    // Turning the QR off stops asking for one.
    fireEvent.click(screen.getByRole("switch", { name: "QR code that opens it in Platter" }));
    expect(screen.getByRole("switch", { name: "QR code that opens it in Platter" })).toHaveAttribute("aria-checked", "false");
  });

  it("released music gets the song.link line; unreleased music never does", async () => {
    const linkSpy = vi.spyOn(backend, "sharePublicLink");
    const { unmount } = render(<ShareCardHost />);
    act(() => openShareCard({ kind: "track", id: released.id, title: released.title, subtitle: "x" }));
    expect(await screen.findByRole("switch", { name: "song.link line" })).toHaveAttribute("aria-checked", "true");
    await waitFor(() => expect(linkSpy).toHaveBeenCalledWith("track", released.id));
    act(() => useShareCard.getState().close());
    unmount();
    linkSpy.mockClear();

    render(<ShareCardHost />);
    act(() => openShareCard({ kind: "track", id: demoTrack.id, title: demoTrack.title, subtitle: "x" }));
    expect(await screen.findByText(/Unreleased music never gets a song.link line/)).toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "song.link line" })).toBeNull();
    expect(linkSpy).not.toHaveBeenCalled();
  });

  it("the settings set the starting choices", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, shareCardQr: false, shareCardSongLink: false } });
    const qrSpy = vi.spyOn(backend, "shareCardQr");
    const linkSpy = vi.spyOn(backend, "sharePublicLink");
    render(<ShareCardHost />);
    act(() => openShareCard({ kind: "track", id: released.id, title: released.title, subtitle: "x" }));
    expect(await screen.findByRole("switch", { name: "QR code that opens it in Platter" })).toHaveAttribute("aria-checked", "false");
    expect(await screen.findByRole("switch", { name: "song.link line" })).toHaveAttribute("aria-checked", "false");
    expect(qrSpy).not.toHaveBeenCalled();
    expect(linkSpy).not.toHaveBeenCalled();
  });
});

describe("lyric cards", () => {
  const words = fx.tracks.find((t) => mockLyricsKind(t.id) === "words" && t.durationMs > 120_000)!;

  it("Share lyrics picks up to 4 lines, then opens a lyric card with them", async () => {
    clearLyricsCache();
    const seek = vi.spyOn(backend, "playSeek");
    playing(words, 40_000);
    render(<LyricsPane />);
    fireEvent.click(await screen.findByRole("button", { name: "Share lyrics" }));
    const view = screen.getByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    const lines = [...view.querySelectorAll<HTMLButtonElement>("button.lyrics__line:not(.lyrics__line--gap)")];
    for (const l of lines.slice(0, 5)) fireEvent.click(l);
    expect(seek).not.toHaveBeenCalled();
    expect(lines.slice(0, 4).every((l) => l.getAttribute("aria-pressed") === "true")).toBe(true);
    expect(lines[4]).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByText("4 of 4 lines")).toBeInTheDocument();
    // Picking one again drops it.
    fireEvent.click(lines[1]);
    expect(screen.getByText("3 of 4 lines")).toBeInTheDocument();
    fireEvent.click(within(screen.getByRole("group", { name: "Pick lyrics to share" })).getByRole("button", { name: "Share lyrics" }));
    const s = useShareCard.getState().subject!;
    expect(s.kind).toBe("track");
    expect(s.lyrics).toEqual([lines[0], lines[2], lines[3]].map((l) => l.getAttribute("aria-label")));
    expect(useLyricPick.getState().picking).toBe(false);
  });

  it("Cancel goes back to seeking", async () => {
    clearLyricsCache();
    playing(words, 40_000);
    render(<LyricsPane />);
    fireEvent.click(await screen.findByRole("button", { name: "Share lyrics" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(useLyricPick.getState().picking).toBe(false);
    expect(screen.getByRole("button", { name: "Share lyrics" })).toBeInTheDocument();
  });
});
