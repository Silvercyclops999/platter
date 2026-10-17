import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Dialog } from "../components/Dialog";

function Form() {
  return (
    <Dialog open onClose={() => {}} label="Edit">
      <button type="button">First</button>
      <select aria-label="Off" disabled>
        <option>a</option>
      </select>
      <textarea aria-label="Also off" disabled />
      <div style={{ display: "none" }}>
        <button type="button">Hidden by its parent</button>
      </div>
      <button type="button" hidden>
        Hidden
      </button>
      <input aria-label="Name" />
      <button type="button">Last</button>
    </Dialog>
  );
}

describe("dialog focus trap", () => {
  it("Tab wraps from the last control to the first, skipping disabled and hidden ones", async () => {
    const user = userEvent.setup();
    render(<Form />);
    await waitFor(() => expect(screen.getByRole("button", { name: "First" })).toHaveFocus());
    await user.tab();
    expect(screen.getByRole("textbox", { name: "Name" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
    await user.tab({ shift: true });
    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
  });

  it("with focus on the dialog itself, Tab and Shift+Tab stay inside", async () => {
    const user = userEvent.setup();
    render(<Form />);
    const dialog = screen.getByRole("dialog");
    await waitFor(() => expect(screen.getByRole("button", { name: "First" })).toHaveFocus());
    dialog.focus();
    await user.tab({ shift: true });
    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
    dialog.focus();
    await user.tab();
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
  });

  it("a dialog with nothing to focus keeps focus on itself", async () => {
    const user = userEvent.setup();
    render(
      <Dialog open onClose={() => {}} label="Note">
        <p>Just text</p>
      </Dialog>,
    );
    const dialog = screen.getByRole("dialog");
    await waitFor(() => expect(dialog).toHaveFocus());
    await user.tab();
    expect(dialog).toHaveFocus();
  });
});
