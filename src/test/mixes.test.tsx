import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type MixSummary } from "../lib/types";
import { HomePage } from "../pages/HomePage";
import { MixArt } from "../pages/MixCard";
import { MixSleeve } from "../pages/MixesPage";
import { MixPage } from "../pages/MixPage";
import { PlayControls } from "../shell/PlayerBar";
import { useApp } from "../stores/app";
import { connectPlayer, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";

const mock = backend as MockBackend;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", avatar: null, server: null, needsServerPick: false, reauthRequired: false } });
  useRouter.getState().reset();
});
afterEach(() => vi.restoreAllMocks());

const route = () => useRouter.getState().entries[useRouter.getState().index].route;

describe("home mix shelves", () => {
  it("shows Made for you with Daily Mixes and the daylist, and mood and genre shelves", async () => {
    render(<HomePage />);
    const shelf = (await screen.findByRole("heading", { name: "Made for you" })).closest("section")!;
    const cards = within(shelf as HTMLElement);
    expect(cards.getByRole("button", { name: "Daily Mix 1" })).toBeInTheDocument();
    expect(cards.getByRole("button", { name: "daylist" })).toBeInTheDocument();
    expect(cards.getByText("Your day in a playlist.")).toBeInTheDocument();
    // Top Songs lists stay off Made for you (search and the profile have them).
    expect(cards.queryByRole("button", { name: "All-Time Top Songs" })).not.toBeInTheDocument();
    // "A, B, C and more" under a Daily Mix.
    expect(within(screen.getByTestId("mix-card-daily:1")).getByText(/ and more$/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Your mixes by mood" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Your genre mixes" })).toBeInTheDocument();
  });

  it("a card opens its mix page and its play button plays the mix", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(backend, "playContext");
    render(<HomePage />);
    await user.click(await screen.findByRole("button", { name: "Daily Mix 2" }));
    expect(route()).toEqual({ name: "mix", id: "daily:2" });
    await user.click(screen.getByRole("button", { name: "Play Daily Mix 2" }));
    expect(play).toHaveBeenCalledWith(expect.objectContaining({ context: { kind: "mix", id: "daily:2", trackIds: undefined } }));
  });

  it("the setting hides the mix shelves", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, homeMixes: false } });
    render(<HomePage />);
    await screen.findByRole("heading", { name: "Recently played" });
    expect(screen.queryByRole("heading", { name: "Made for you" })).toBeNull();
  });
});

describe("mix art", () => {
  it("a photo that fails to load falls back to the cover collage", () => {
    const mix: MixSummary = {
      id: "daily:1",
      kind: "daily",
      title: "Daily Mix 1",
      description: "",
      color: "#336699",
      art: "0a1b2c3d:/library/metadata/5/thumb/1",
      covers: ["0a1b2c3d:/library/metadata/6/thumb/1"],
      artistIds: [],
      tag: null,
      trackCount: 10,
      durationMs: 1,
      nextUpdateAt: null,
    };
    const { container } = render(<MixArt mix={mix} />);
    const photo = container.querySelector("img.mix-art__photo");
    expect(photo).not.toBeNull();
    fireEvent.error(photo!);
    expect(container.querySelector("img.mix-art__photo")).toBeNull();
    expect(container.querySelector(".mix-art__collage")).not.toBeNull();
  });
});

describe("a mix's picture in the list", () => {
  const mix: MixSummary = {
    id: "daily:1",
    kind: "daily",
    title: "Daily Mix 1",
    description: "Copperline Choir, Motte and more",
    color: "#336699",
    art: "0a1b2c3d:/library/metadata/5/thumb/1",
    covers: ["0a1b2c3d:/library/metadata/6/thumb/1", "0a1b2c3d:/library/metadata/7/thumb/1"],
    artistIds: [],
    tag: null,
    trackCount: 10,
    durationMs: 1,
    nextUpdateAt: null,
  };
  const pictures = (c: HTMLElement) => [...c.querySelectorAll("img")].map((n) => n.getAttribute("src") ?? "");

  it("is the leading artist's photo, with no printed name (the row prints it)", () => {
    const { container } = render(<MixSleeve mix={mix} size={56} />);
    expect(pictures(container)).toHaveLength(1);
    expect(container.querySelector(".mix-art__collage")).toBeNull();
    expect(container.querySelector(".mix-art__strip")).toBeNull();
  });

  it("without a photo is four of its covers", () => {
    const collage = render(<MixSleeve mix={{ ...mix, art: null }} size={56} />);
    expect(collage.container.querySelectorAll(".mix-art__collage .art")).toHaveLength(4);
    expect(collage.container.textContent).toBe("");
  });

  it("Top Songs shows its covers, not one artist", () => {
    const { container } = render(<MixSleeve mix={{ ...mix, kind: "topSongs" }} size={56} />);
    expect(container.querySelectorAll(".mix-art__collage .art")).toHaveLength(4);
  });
});

describe("mix page", () => {
  it("shows Made for <user> with the song count, saves as a playlist and plays", async () => {
    const user = userEvent.setup();
    const create = vi.spyOn(backend, "playlistCreate");
    const play = vi.spyOn(backend, "playContext");
    render(
      <>
        <MixPage id="daily:1" />
        <ContextMenuHost />
      </>,
    );
    expect(await screen.findByRole("heading", { level: 1, name: "Daily Mix 1" })).toBeInTheDocument();
    const page = (await mock.mixPage("daily:1"))!;
    expect(screen.getByText("Made for listener")).toBeInTheDocument();
    expect(screen.getByText(`${page.tracks.length} songs,`)).toBeInTheDocument();
    // Daily Mixes have no steering chips.
    expect(screen.queryByRole("button", { name: "More familiar" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "More options for Daily Mix 1" }));
    await user.click(await screen.findByRole("menuitem", { name: "Save as playlist" }));
    await waitFor(() => expect(create).toHaveBeenCalledWith("Daily Mix 1", page.tracks.map((t) => t.id)));
    await user.click(screen.getByRole("button", { name: "Play Daily Mix 1" }));
    expect(play).toHaveBeenCalledWith(expect.objectContaining({ context: { kind: "mix", id: "daily:1", trackIds: undefined } }));
  });

  it("genre mixes have steering chips that play the steered songs", async () => {
    const user = userEvent.setup();
    const shelves = await mock.mixesHome();
    const genre = shelves.genres[0];
    const play = vi.spyOn(backend, "playContext");
    const steer = vi.spyOn(backend, "mixPage");
    render(<MixPage id={genre.id} />);
    await screen.findByRole("heading", { level: 1, name: genre.title });
    await user.click(screen.getByRole("button", { name: "More familiar" }));
    await waitFor(() => expect(steer).toHaveBeenCalledWith(genre.id, "familiar"));
    await waitFor(() => expect(screen.getByRole("button", { name: "More familiar" })).toHaveAttribute("aria-pressed", "true"));
    await user.click(screen.getByRole("button", { name: `Play ${genre.title}` }));
    const args = play.mock.calls[play.mock.calls.length - 1][0];
    expect(args.context.kind).toBe("mix");
    expect(args.context.trackIds?.length).toBeGreaterThan(0);
  });

  it("the daylist shows when it next updates", async () => {
    render(<MixPage id="daylist" />);
    expect(await screen.findByText(/^Next update at /)).toBeInTheDocument();
    expect(screen.getByText(/^Here's some .*, inspired by your listening on /)).toBeInTheDocument();
  });

  it("All-Time Top Songs shows play counts", async () => {
    render(<MixPage id="topSongs" />);
    await screen.findByRole("heading", { level: 1, name: "All-Time Top Songs" });
    const top = (await mock.mixPage("topSongs"))!.tracks[0];
    expect(document.querySelector(".track-row--plays .track-row__plays")?.textContent).toBe(top.viewCount.toLocaleString());
  });

  it("smooth transitions can be turned on for this playback from the menu", async () => {
    const user = userEvent.setup();
    const disconnect = connectPlayer(mock);
    const smooth = vi.spyOn(backend, "setSmoothTransitions");
    render(
      <>
        <MixPage id="daily:1" />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("heading", { level: 1, name: "Daily Mix 1" });
    await user.click(screen.getByRole("button", { name: "Play Daily Mix 1" }));
    await waitFor(() => expect(usePlayer.getState().state?.queue.context?.kind).toBe("mix"));
    await user.click(screen.getByRole("button", { name: "More options for Daily Mix 1" }));
    await user.click(await screen.findByRole("menuitem", { name: "Turn on smooth transitions" }));
    expect(smooth).toHaveBeenCalledWith(true);
    await waitFor(() => expect(usePlayer.getState().state?.smoothTransitions).toBe(true));
    disconnect();
  });
});

describe("a mix page", () => {
  it("has no sentence about the numbers and no chart: the songs follow the buttons", async () => {
    render(<MixPage id="daylist" />);
    await screen.findAllByRole("row");
    expect(screen.queryByTestId("mix-story")).toBeNull();
    expect(screen.queryByText(/you've hardly played/)).toBeNull();
    expect(document.querySelector("figure")).toBeNull();
  });
});

describe("radio", () => {
  it("Go to song radio opens a radio page that starts with the song", async () => {
    const user = userEvent.setup();
    const t = mock.fx.tracks[0];
    render(
      <>
        <div role="grid">
          <TrackRow track={t} index={1} onPlay={() => {}} />
        </div>
        <ContextMenuHost />
      </>,
    );
    await user.pointer({ keys: "[MouseRight]", target: screen.getByText(t.title, { selector: ".track-row__title *, .track-row__title" }) });
    await user.click(await screen.findByRole("menuitem", { name: "Go to song radio" }));
    expect(route()).toEqual({ name: "radio", seed: `track:${t.id}` });
  });

  it("the radio page has the seed first, a 'with' line and plays its songs as an endless radio", async () => {
    const user = userEvent.setup();
    const t = mock.fx.tracks[0];
    const play = vi.spyOn(backend, "playContext");
    render(<MixPage seed={`track:${t.id}`} />);
    expect(await screen.findByRole("heading", { level: 1, name: `${t.title} Radio` })).toBeInTheDocument();
    expect(screen.getByText(/^with .* and more$/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "More new" })).toBeInTheDocument();
    expect(document.querySelectorAll(".mix-circles__item")).toHaveLength(3);
    const rows = document.querySelectorAll(".track-row");
    expect(rows[0].textContent).toContain(t.title);
    await user.click(screen.getByRole("button", { name: `Play ${t.title} Radio` }));
    const args = play.mock.calls[play.mock.calls.length - 1][0];
    expect(args.context.kind).toBe("radio");
    expect(args.context.id).toBe(`track:${t.id}`);
    expect(args.context.trackIds?.[0]).toBe(t.id);
  });
});

describe("smart shuffle", () => {
  it("isn't offered on an album: shuffle is just on or off", async () => {
    const user = userEvent.setup();
    const disconnect = connectPlayer(mock);
    await mock.playContext({ context: { kind: "album", id: mock.fx.albums[1].id } });
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, smartShuffle: true } });
    render(<PlayControls />);
    await user.click(await screen.findByRole("button", { name: "Enable shuffle" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Disable shuffle" })).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Disable shuffle" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Enable shuffle" })).toBeInTheDocument());
    expect(usePlayer.getState().state?.smartShuffle).toBe(false);
    disconnect();
  });
});
