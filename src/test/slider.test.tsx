import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Slider } from "../components/Slider";

describe("Slider", () => {
  it("exposes slider semantics", () => {
    render(<Slider label="Volume" value={0.5} max={1} step={0.05} valueText={(v) => `${Math.round(v * 100)}%`} />);
    const s = screen.getByRole("slider", { name: "Volume" });
    expect(s).toHaveAttribute("aria-valuemin", "0");
    expect(s).toHaveAttribute("aria-valuemax", "1");
    expect(s).toHaveAttribute("aria-valuenow", "0.5");
    expect(s).toHaveAttribute("aria-valuetext", "50%");
    expect(s).toHaveAttribute("tabindex", "0");
  });

  it("moves with arrows, PageUp/PageDown, Home and End", async () => {
    const user = userEvent.setup();
    const onCommit = vi.fn();
    const { rerender } = render(<Slider label="Seek" value={50} max={100} step={5} onCommit={onCommit} />);
    const s = screen.getByRole("slider", { name: "Seek" });
    s.focus();

    await user.keyboard("{ArrowRight}");
    expect(onCommit).toHaveBeenLastCalledWith(55, "key");
    await user.keyboard("{ArrowLeft}");
    expect(onCommit).toHaveBeenLastCalledWith(45, "key");
    await user.keyboard("{ArrowUp}");
    expect(onCommit).toHaveBeenLastCalledWith(55, "key");
    await user.keyboard("{PageUp}");
    expect(onCommit).toHaveBeenLastCalledWith(60, "key");
    await user.keyboard("{PageDown}");
    expect(onCommit).toHaveBeenLastCalledWith(40, "key");
    await user.keyboard("{Home}");
    expect(onCommit).toHaveBeenLastCalledWith(0, "key");
    await user.keyboard("{End}");
    expect(onCommit).toHaveBeenLastCalledWith(100, "key");

    // Clamped at the ends.
    rerender(<Slider label="Seek" value={100} max={100} step={5} onCommit={onCommit} />);
    await user.keyboard("{ArrowRight}");
    expect(onCommit).toHaveBeenLastCalledWith(100, "key");
  });

  it("uses a custom Page Up/Down step", async () => {
    const user = userEvent.setup();
    const onCommit = vi.fn();
    render(<Slider label="Seek" value={60_000} max={200_000} step={5_000} pageStep={25_000} onCommit={onCommit} />);
    screen.getByRole("slider", { name: "Seek" }).focus();
    await user.keyboard("{PageUp}");
    expect(onCommit).toHaveBeenLastCalledWith(85_000, "key");
    await user.keyboard("{PageDown}");
    expect(onCommit).toHaveBeenLastCalledWith(35_000, "key");
  });

  it("commits on press too with commitOnPress", () => {
    const onCommit = vi.fn();
    render(<Slider label="Seek" value={0} max={200} commitOnPress onCommit={onCommit} />);
    const s = screen.getByRole("slider", { name: "Seek" });
    const track = s.querySelector(".slider__track") as HTMLElement;
    track.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, height: 4, right: 100, bottom: 4, x: 0, y: 0, toJSON() {} });
    fireEvent.pointerDown(s, { button: 0, clientX: 25, pointerId: 1 });
    expect(onCommit).toHaveBeenCalledWith(50, "press");
    fireEvent.pointerCancel(s, { pointerId: 1 });
    expect(onCommit).toHaveBeenLastCalledWith(50, "release");
  });

  it("ignores keys when disabled", () => {
    const onCommit = vi.fn();
    render(<Slider label="Seek" value={10} max={100} disabled onCommit={onCommit} />);
    const s = screen.getByRole("slider", { name: "Seek" });
    expect(s).toHaveAttribute("tabindex", "-1");
    fireEvent.keyDown(s, { key: "ArrowRight" });
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("commits the pointer position on release", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(<Slider label="Seek" value={0} max={200} onChange={onChange} onCommit={onCommit} />);
    const s = screen.getByRole("slider", { name: "Seek" });
    const track = s.querySelector(".slider__track") as HTMLElement;
    track.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, height: 4, right: 100, bottom: 4, x: 0, y: 0, toJSON() {} });
    fireEvent.pointerDown(s, { button: 0, clientX: 25, pointerId: 1 });
    expect(onChange).toHaveBeenLastCalledWith(50);
    fireEvent.pointerMove(s, { clientX: 75, pointerId: 1 });
    expect(onChange).toHaveBeenLastCalledWith(150);
    expect(onCommit).not.toHaveBeenCalled();
    fireEvent.pointerUp(s, { clientX: 75, pointerId: 1 });
    expect(onCommit).toHaveBeenCalledWith(150, "release");
  });
});

describe("Slider drag across a song change", () => {
  const box = (s: HTMLElement) => {
    const track = s.querySelector(".slider__track") as HTMLElement;
    track.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, height: 4, right: 100, bottom: 4, x: 0, y: 0, toJSON() {} });
  };

  it("drops the drag when the song changes, so letting go doesn't seek the new song", () => {
    const onCommit = vi.fn();
    const { rerender } = render(<Slider label="Seek" value={0} max={200} resetKey="a" onCommit={onCommit} />);
    const s = screen.getByRole("slider", { name: "Seek" });
    box(s);
    fireEvent.pointerDown(s, { button: 0, clientX: 25, pointerId: 1 });
    fireEvent.pointerMove(s, { clientX: 60, pointerId: 1 });
    expect(s).toHaveAttribute("aria-valuenow", "120");
    // Same length, new song.
    rerender(<Slider label="Seek" value={3} max={200} resetKey="b" onCommit={onCommit} />);
    expect(s).toHaveAttribute("aria-valuenow", "3");
    fireEvent.pointerUp(s, { clientX: 60, pointerId: 1 });
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("drops the drag when the range changes", () => {
    const onCommit = vi.fn();
    const { rerender } = render(<Slider label="Seek" value={0} max={200} onCommit={onCommit} />);
    const s = screen.getByRole("slider", { name: "Seek" });
    box(s);
    fireEvent.pointerDown(s, { button: 0, clientX: 25, pointerId: 1 });
    rerender(<Slider label="Seek" value={0} max={300} onCommit={onCommit} />);
    fireEvent.pointerUp(s, { clientX: 60, pointerId: 1 });
    expect(onCommit).not.toHaveBeenCalled();
  });
});
