/**
 * Find a song by a line of its lyrics: the lyrics stored here are searched
 * for three or more words (or words in quotes), and the matching line shows
 * under the song, on the results page, under Songs and on the phone.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { lyricQuery, mockLyricSearch } from "../lib/mock/lyricSearch";
import { DEFAULT_SETTINGS } from "../lib/types";
import { PhoneSearch } from "../phone/pages/Search";
import { SearchPage } from "../pages/SearchPage";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const RIVER = "[00:01.00]We walked along the water\n[00:05.00]And the river took the light\n[00:09.00]Away from us tonight";
const LINE = "“And the river took the light”";

beforeEach(async () => {
  useUi.setState({ menu: null });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useRouter.getState().reset({ name: "home" });
  for (const id of await backend.ownLyricsIds()) await backend.ownLyricsDelete(id);
});

const song = () => mock.fx.tracks[5];

describe("lyric search", () => {
  it("finds a song by a line of its lyrics and shows the line", async () => {
    await backend.ownLyricsSave(song().id, RIVER, true);
    const user = userEvent.setup();
    render(<SearchPage query="the river took" />);
    const list = await screen.findByRole("list", { name: "Results" });
    const row = within(list).getByRole("listitem", { name: new RegExp(`^${song().title},`) });
    expect(row).toHaveTextContent(LINE);
    await user.click(screen.getByRole("button", { name: "Songs" }));
    const songRow = await screen.findByRole("row", { name: new RegExp(song().title) });
    expect(songRow).toHaveTextContent(LINE);
  });

  it("shows the line on the phone too", async () => {
    await backend.ownLyricsSave(song().id, RIVER, true);
    useRouter.getState().reset({ name: "search", query: "the river took" });
    render(<PhoneSearch />);
    const results = await screen.findByRole("region", { name: "Results" }, { timeout: 4000 });
    expect(await within(results).findByText(LINE)).toBeInTheDocument();
  });

  it("short searches don't show lyric matches", async () => {
    await backend.ownLyricsSave(song().id, RIVER, true);
    const r = await backend.search({ query: "river took", filter: "all" });
    expect(r.lyricLines).toBeUndefined();
    const quoted = await backend.search({ query: '"river took"', filter: "all" });
    expect(quoted.lyricLines?.[song().id]).toBe("And the river took the light");
  });

  it("songs found by name come first", async () => {
    const named = mock.fx.tracks[0];
    await backend.ownLyricsSave(song().id, `[00:01.00]sing ${named.title.toLowerCase()} again`, true);
    const r = await backend.search({ query: `"${named.title}"`, filter: "songs" });
    const ids = r.songs.map((t) => t.id);
    expect(ids.indexOf(named.id)).toBeLessThan(ids.indexOf(song().id));
    expect(r.lyricLines?.[named.id]).toBeUndefined();
  });

  it("reads queries the way the core does", () => {
    expect(lyricQuery("river light")).toBeNull();
    expect(lyricQuery('"river"')).toEqual({ words: ["river"], prefix: false });
    expect(lyricQuery("the river to")).toEqual({ words: ["the", "river", "to"], prefix: true });
    expect(mockLyricSearch("the light away", [["1", RIVER]])).toEqual([{ trackId: "1", line: "And the river took the light / Away from us tonight" }]);
  });
});
