/** Which year's Wrapped is open, if any (it fills the window over everything). */
import { create } from "zustand";

interface WrappedStore {
  year: number | null;
  open: (year: number) => void;
  close: () => void;
}

export const useWrapped = create<WrappedStore>()((set) => ({
  year: null,
  open: (year) => set({ year }),
  close: () => set({ year: null }),
}));

export const wrapped = {
  open: (year: number) => useWrapped.getState().open(year),
  close: () => useWrapped.getState().close(),
};
