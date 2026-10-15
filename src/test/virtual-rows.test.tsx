import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { VIRTUAL_ROWS_FROM, VirtualRows } from "../components/VirtualRows";

function list(count: number) {
  return render(
    <div className="main__scroll" style={{ height: 900, overflow: "auto" }}>
      <ul>
        <VirtualRows
          count={count}
          rowHeight={60}
          renderRow={(i) => (
            <li key={i} className="row">
              Row {i + 1}
            </li>
          )}
        />
      </ul>
    </div>,
  );
}

describe("VirtualRows", () => {
  it("renders a short list whole", () => {
    const { container } = list(50);
    expect(container.querySelectorAll("li.row")).toHaveLength(50);
    expect(container.querySelector(".virtual-spacer")).toBeNull();
  });

  it("renders only the rows near the viewport of a long list, keeping its height", () => {
    const count = 100_000;
    expect(count).toBeGreaterThan(VIRTUAL_ROWS_FROM);
    const { container } = list(count);
    const rows = container.querySelectorAll("li.row");
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(100);
    expect(rows[0].textContent).toBe("Row 1");
    const spacers = container.querySelectorAll<HTMLElement>(".virtual-spacer");
    expect(spacers).toHaveLength(2);
    // The rows not drawn still take their space, so the scrollbar is right.
    const after = parseFloat(spacers[1].style.height);
    expect(after).toBe((count - rows.length) * 60);
  });
});
