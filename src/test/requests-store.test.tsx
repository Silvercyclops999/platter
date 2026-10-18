/**
 * The shared request list Get buttons read: waiting requests are followed
 * too, a read that started before a change doesn't undo it, and a first
 * read that failed is tried again. The core's list is faked per test.
 */
import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { LidarrRequest, LidarrRequestState } from "../lib/types";
import { FOLLOW_MS, RETRY_MS, refreshRequests, resetRequests, upsertRequest, useRequestFor, useRequests } from "../stores/requests";

const RG = "10000000-0000-0000-0000-00000000aaaa";

function req(state: LidarrRequestState, id = "r1"): LidarrRequest {
  return {
    id,
    releaseGroupId: RG,
    title: "Low Tide Radio",
    artist: "Cold Coast",
    artistId: null,
    state,
    destination: null,
    lidarrAlbumId: null,
    addedArtist: false,
    alreadyInLidarr: false,
    createdAt: 1,
    updatedAt: 1,
    message: null,
    notes: [],
    auto: false,
    protected: false,
  };
}

function Watch() {
  const r = useRequestFor(RG);
  return <span data-testid="state">{r?.state ?? "none"}</span>;
}

beforeEach(() => {
  resetRequests();
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("the request list", () => {
  it("keeps reading while a request waits to send", async () => {
    vi.useFakeTimers();
    const reads = vi.spyOn(backend, "lidarrRequests").mockResolvedValue([req("queued")]);
    render(<Watch />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(reads).toHaveBeenCalledTimes(1);
    reads.mockResolvedValue([req("searching")]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(FOLLOW_MS);
    });
    expect(reads).toHaveBeenCalledTimes(2);
    expect(useRequests.getState().list[0].state).toBe("searching");
  });

  it("a read that started before a Get doesn't put the button back", async () => {
    let answer: (l: LidarrRequest[]) => void = () => {};
    vi.spyOn(backend, "lidarrRequests").mockImplementation(() => new Promise((res) => (answer = res)));
    const reading = refreshRequests();
    // The Get lands while the read is out; the read answers with the list from before it.
    upsertRequest(req("searching"));
    answer([]);
    await reading;
    expect(useRequests.getState().list.map((r) => r.state)).toEqual(["searching"]);
    // A read that starts after it takes over as usual.
    vi.spyOn(backend, "lidarrRequests").mockResolvedValue([req("downloading")]);
    await refreshRequests();
    expect(useRequests.getState().list.map((r) => r.state)).toEqual(["downloading"]);
  });

  it("a first read that failed is tried again", async () => {
    vi.useFakeTimers();
    const reads = vi.spyOn(backend, "lidarrRequests").mockRejectedValueOnce({ kind: "internal", message: "busy" }).mockResolvedValue([req("searching")]);
    const view = render(<Watch />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(reads).toHaveBeenCalledTimes(1);
    expect(useRequests.getState().loaded).toBe(false);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RETRY_MS);
    });
    expect(reads).toHaveBeenCalledTimes(2);
    expect(view.getByTestId("state").textContent).toBe("searching");
  });
});

describe("a Get that's still undoable", () => {
  it("is sent when its button goes away", async () => {
    const { GetButton } = await import("../pages/GetButton");
    const { useSettings } = await import("../stores/settings");
    useSettings.setState({ settings: { askBeforeGet: false } as never });
    vi.spyOn(backend, "lidarrRequests").mockResolvedValue([]);
    const get = vi.spyOn(backend, "lidarrGet").mockResolvedValue({ record: req("searching"), needs: null });
    const view = render(<GetButton album={{ releaseGroupId: RG, title: "Low Tide Radio", artist: "Cold Coast" }} lidarrReady />);
    act(() => view.getByRole("button", { name: "Get Low Tide Radio" }).click());
    expect(view.getByRole("button", { name: "Undo Get Low Tide Radio" })).toBeTruthy();
    view.unmount();
    await waitFor(() => expect(get).toHaveBeenCalledWith(expect.objectContaining({ releaseGroupId: RG })));
    await waitFor(() => expect(useRequests.getState().list.map((r) => r.state)).toEqual(["searching"]));
  });
});
