/**
 * Looked-up artist photos: a reused card never shows the last artist's
 * photo, and an empty answer (offline) is asked again next time.
 */
import { renderHook, waitFor } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { resetArtistPhotos, useFeaturedPhoto } from "../lib/artistPhotos";

beforeEach(() => resetArtistPhotos());
afterEach(() => vi.restoreAllMocks());

it("drops the last artist's photo as soon as the card shows another artist", async () => {
  const answers: Record<string, (p: { photo: string | null; credit: null }) => void> = {};
  vi.spyOn(backend, "featuredArtistPhoto").mockImplementation((id) => new Promise((r) => (answers[id] = r)));
  const ref = createRef<HTMLElement>();
  const { result, rerender } = renderHook(({ id, name }) => useFeaturedPhoto(ref, id, name, null), { initialProps: { id: "a", name: "First" } });
  await waitFor(() => expect(answers.a).toBeDefined());
  answers.a({ photo: "first.jpg", credit: null });
  await waitFor(() => expect(result.current).toBe("first.jpg"));
  // The same card now shows a different artist whose photo is still on its way.
  rerender({ id: "b", name: "Second" });
  expect(result.current).toBeNull();
  await waitFor(() => expect(answers.b).toBeDefined());
  answers.b({ photo: "second.jpg", credit: null });
  await waitFor(() => expect(result.current).toBe("second.jpg"));
});

it("asks again after an empty answer instead of keeping it for the session", async () => {
  const look = vi.spyOn(backend, "featuredArtistPhoto").mockResolvedValueOnce({ photo: null, credit: null }).mockResolvedValue({ photo: "back.jpg", credit: null });
  const ref = createRef<HTMLElement>();
  const first = renderHook(() => useFeaturedPhoto(ref, "c", "Offline", null));
  await waitFor(() => expect(look).toHaveBeenCalledTimes(1));
  expect(first.result.current).toBeNull();
  first.unmount();
  const again = renderHook(() => useFeaturedPhoto(ref, "c", "Offline", null));
  await waitFor(() => expect(again.result.current).toBe("back.jpg"));
  expect(look).toHaveBeenCalledTimes(2);
});
