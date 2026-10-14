/** Track list rows for tests: the grid's rows without its column header row. */
export const body = (rows: HTMLElement[]): HTMLElement[] => rows.filter((r) => !r.classList.contains("track-list__head"));
