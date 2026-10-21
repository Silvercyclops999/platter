/** Picks an item from the Lyrics options menu under the lyrics. */
import { act, fireEvent, screen } from "@testing-library/react";
import { OPTIONS_LABEL } from "../shell/LyricsTrust";
import { useUi } from "../stores/ui";

/** The labels the options menu has now. */
export async function lyricsOptions(): Promise<string[]> {
  fireEvent.click(await screen.findByRole("button", { name: OPTIONS_LABEL }));
  const items = useUi.getState().menu?.items ?? [];
  act(() => useUi.setState({ menu: null }));
  return items.map((i) => i.label);
}

/** Opens the options menu and chooses an item by its label. */
export async function chooseLyricsOption(label: string | RegExp) {
  fireEvent.click(await screen.findByRole("button", { name: OPTIONS_LABEL }));
  const item = useUi.getState().menu?.items.find((i) => (typeof label === "string" ? i.label === label : label.test(i.label)));
  if (!item) throw new Error(`No "${label}" in Lyrics options: ${useUi.getState().menu?.items.map((i) => i.label).join(", ")}`);
  act(() => {
    useUi.setState({ menu: null });
    item.onSelect!();
  });
}
