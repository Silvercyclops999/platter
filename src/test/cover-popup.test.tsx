import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { COVER_MAX, coverSize } from "../components/CoverPopup";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { AlbumPage } from "../pages/AlbumPage";

const mock = backend as MockBackend;
const album = () => mock.fx.albums[0];

async function openPopup() {
  const user = userEvent.setup();
  render(<AlbumPage id={album().id} />);
  const cover = await screen.findByRole("button", { name: "View the cover art" });
  await user.click(cover);
  const dialog = await screen.findByRole("dialog", { name: `${album().title} cover` });
  return { user, cover, dialog };
}

describe("album cover popup", () => {
  it("sizes the cover to the window, up to the largest size", () => {
    expect(coverSize(2560, 1440)).toBe(COVER_MAX);
    expect(coverSize(800, 600)).toBe(600 - 96 - 56);
    expect(coverSize(500, 1400)).toBe(500 - 96);
  });

  it("opens from the header cover with focus on Close, and Tab stays there", async () => {
    const { user } = await openPopup();
    const close = screen.getByRole("button", { name: "Close" });
    await waitFor(() => expect(close).toHaveFocus());
    await user.tab();
    expect(close).toHaveFocus();
    await user.tab({ shift: true });
    expect(close).toHaveFocus();
  });

  it("closes on Escape and returns focus to the cover", async () => {
    const { user, cover } = await openPopup();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(cover).toHaveFocus();
  });

  it("closes on the scrim and the Close button, but not on the cover", async () => {
    const { user, dialog } = await openPopup();
    await user.click(dialog.querySelector(".cover-popup__cover img, .cover-popup__cover .art")!);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.click(dialog);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await user.click(screen.getByRole("button", { name: "View the cover art" }));
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("requests the art at the shown size", async () => {
    const { dialog } = await openPopup();
    const box = dialog.querySelector<HTMLElement>(".cover-popup__cover")!;
    const size = coverSize(window.innerWidth, window.innerHeight);
    expect(box.style.width).toBe(`${size}px`);
    expect(dialog.querySelector(".cover-popup__cover .art")).toHaveStyle({ width: `${size}px` });
  });
});
