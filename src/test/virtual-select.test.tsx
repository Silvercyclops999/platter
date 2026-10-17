import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { TrackList, TrackRow } from "../components/TrackRow";
import { resetTypeAhead } from "../components/listKeys";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Track } from "../lib/types";
import { useUi } from "../stores/ui";
import { body } from "./rows";

const mock = backend as MockBackend;
const base = mock.fx.tracks[0];
const TITLES = ["Apple", "Banana", "Cherry", "Damson", "Elder", "Fig"];
const tracks: Track[] = Array.from({ length: 300 }, (_, i) => ({ ...base, id: `v${i}`, title: `${TITLES[i % TITLES.length]} ${i}` }));
const model = { keys: tracks.map((t) => t.id), tracks };

/** A list that mounts only a window of 10 rows, like a virtual list. */
function Windowed() {
  const [first, setFirst] = useState(0);
  const shown = tracks.slice(first, first + 10);
  return (
    <>
      <TrackList label="Songs" rowCount={tracks.length} model={model} scrollToIndex={(i) => setFirst(Math.max(0, Math.min(i, tracks.length - 10)))}>
        {shown.map((t, k) => (
          <TrackRow key={t.id} track={t} index={first + k + 1} rowIndex={first + k + 2} onPlay={() => {}} />
        ))}
      </TrackList>
      <ContextMenuHost />
    </>
  );
}

const rows = () => body(screen.getAllByRole("row"));

afterEach(() => {
  act(() => useUi.setState({ menu: null }));
  resetTypeAhead();
});

describe("virtual track lists", () => {
  it("Ctrl+A selects every song, not only the ones on the page", async () => {
    const user = userEvent.setup();
    render(<Windowed />);
    await user.click(rows()[0]);
    await user.keyboard("{Control>}a{/Control}");
    await user.pointer({ keys: "[MouseRight]", target: rows()[3] });
    expect(await screen.findByRole("menu", { name: "Options for 300 songs" })).toBeInTheDocument();
  });

  it("End and Home jump to the ends of the whole list and keep focus", async () => {
    const user = userEvent.setup();
    render(<Windowed />);
    await user.click(rows()[0]);
    await user.keyboard("{End}");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-row-key", "v299"));
    expect(document.activeElement).toHaveAttribute("data-selected");
    await user.keyboard("{Home}");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-row-key", "v0"));
  });

  it("Shift ranges count rows past the ones on the page", async () => {
    const user = userEvent.setup();
    render(<Windowed />);
    await user.click(rows()[0]);
    await user.keyboard("{Shift>}");
    await user.click(rows()[2]);
    await user.keyboard("{/Shift}");
    await user.pointer({ keys: "[MouseRight]", target: rows()[1] });
    expect(await screen.findByRole("menu", { name: "Options for 3 songs" })).toBeInTheDocument();
    act(() => useUi.setState({ menu: null }));
    // Shift+Down from the last row on the page carries on past it.
    rows()[9].focus();
    await user.click(rows()[9]);
    for (let i = 0; i < 5; i++) await user.keyboard("{Shift>}{ArrowDown}{/Shift}");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-row-key", "v14"));
    await user.pointer({ keys: "[MouseRight]", target: document.activeElement as HTMLElement });
    const menu = await screen.findByRole("menu", { name: "Options for 6 songs" });
    expect(within(menu).getAllByRole("menuitem").length).toBeGreaterThan(0);
  });

  it("type-ahead finds a song that isn't on the page", async () => {
    const user = userEvent.setup();
    render(<Windowed />);
    await user.click(rows()[0]);
    // Rows 0 to 9 are on the page; "Fig 11" is the second Fig, past them.
    await user.keyboard("f");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-row-key", "v5"));
    await user.keyboard("f");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-row-key", "v11"));
    expect(document.activeElement).toHaveAttribute("data-selected");
  });
});
