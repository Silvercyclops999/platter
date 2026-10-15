import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn(async (..._args: unknown[]) => undefined as unknown);
const listen = vi.fn(async (..._args: unknown[]) => () => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invoke(...a),
  convertFileSrc: (p: string, proto: string) => `http://${proto}.localhost/${encodeURIComponent(p)}`,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listen(...a) }));

const { backend, createTauriBackend, selectBackend, isTauri, artUrl } = await import("../lib/backend");
const { artBucket, artPixelSize, devicePixelRatio } = await import("../lib/artSize");
const { MockBackend } = await import("../lib/mock");

describe("backend selection", () => {
  it("uses the mock outside Tauri", () => {
    expect(isTauri(window)).toBe(false);
    expect(backend.kind).toBe("mock");
    expect(selectBackend({}).kind).toBe("mock");
  });

  it("uses Tauri when __TAURI_INTERNALS__ exists", () => {
    expect(selectBackend({ __TAURI_INTERNALS__: {} }).kind).toBe("tauri");
  });
});

// Every command in docs/IPC.md → how the UI calls it.
const CALLS: [string, (b: ReturnType<typeof createTauriBackend>) => Promise<unknown>, Record<string, unknown> | undefined][] = [
  ["auth_status", (b) => b.authStatus(), undefined],
  ["auth_start", (b) => b.authStart(), undefined],
  ["auth_cancel", (b) => b.authCancel(), undefined],
  ["auth_sign_out", (b) => b.authSignOut(), undefined],
  ["servers_list", (b) => b.serversList(), undefined],
  ["server_select", (b) => b.serverSelect("m1"), { machineId: "m1" }],
  ["sync_status", (b) => b.syncStatus(), undefined],
  ["sync_now", (b) => b.syncNow(true), { full: true }],
  ["home", (b) => b.home(), undefined],
  ["artists", (b) => b.artists({ sort: "name", offset: 0, limit: 50 }), { sort: "name", offset: 0, limit: 50 }],
  ["artist_names", (b) => b.artistNames(), undefined],
  ["albums", (b) => b.albums({ sort: "year", offset: 10, limit: 5, section: "live" }), { sort: "year", offset: 10, limit: 5, section: "live" }],
  ["playlists", (b) => b.playlists(), undefined],
  ["artist_page", (b) => b.artistPage("7"), { id: "7" }],
  ["album_page", (b) => b.albumPage("8"), { id: "8" }],
  ["playlist_page", (b) => b.playlistPage("9"), { id: "9" }],
  ["recent_tracks", (b) => b.recentTracks(20), { limit: 20 }],
  ["recents", (b) => b.recents(100), { limit: 100 }],
  ["now_playing_details", (b) => b.nowPlayingDetails("5"), { trackId: "5" }],
  ["search", (b) => b.search({ query: "q", filter: "songs", limit: 5 }), { query: "q", filter: "songs", limit: 5 }],
  ["search_completions", (b) => b.searchCompletions("lan", 4), { query: "lan", limit: 4 }],
  ["search_recents", (b) => b.searchRecents(), undefined],
  ["search_recent_add", (b) => b.searchRecentAdd("album", "x"), { kind: "album", id: "x" }],
  ["search_recent_remove", (b) => b.searchRecentRemove("track", "y"), { kind: "track", id: "y" }],
  ["search_recents_clear", (b) => b.searchRecentsClear(), undefined],
  [
    "play_context",
    (b) => b.playContext({ context: { kind: "album", id: "1" }, startTrackId: "2", shuffle: true }),
    { context: { kind: "album", id: "1" }, startTrackId: "2", shuffle: true },
  ],
  ["play_toggle", (b) => b.playToggle(), undefined],
  ["play_pause", (b) => b.playPause(), undefined],
  ["play_resume", (b) => b.playResume(), undefined],
  ["play_next", (b) => b.playNext(), undefined],
  ["play_previous", (b) => b.playPrevious(), undefined],
  ["play_seek", (b) => b.playSeek(1234), { positionMs: 1234 }],
  ["set_volume", (b) => b.setVolume(0.5), { volume: 0.5 }],
  ["set_muted", (b) => b.setMuted(true), { muted: true }],
  ["set_shuffle", (b) => b.setShuffle(true), { shuffle: true }],
  ["set_context_shuffle", (b) => b.setContextShuffle("12", true), { contextId: "12", shuffle: true }],
  ["set_context_smart_shuffle", (b) => b.setContextSmartShuffle("12", true), { contextId: "12", on: true }],
  ["smart_suggestion_remove", (b) => b.smartSuggestionRemove(), undefined],
  ["smart_suggestion_keep", (b) => b.smartSuggestionKeep("q4"), { itemId: "q4" }],
  ["set_repeat", (b) => b.setRepeat("one"), { repeat: "one" }],
  ["queue_add", (b) => b.queueAdd(["1", "2"], "next"), { trackIds: ["1", "2"], at: "next" }],
  ["queue_remove", (b) => b.queueRemove("q1"), { itemId: "q1" }],
  ["queue_move", (b) => b.queueMove("q1", 3), { itemId: "q1", toIndex: 3 }],
  ["queue_clear", (b) => b.queueClear(), undefined],
  ["queue_jump", (b) => b.queueJump("q2"), { itemId: "q2" }],
  ["player_state", (b) => b.playerState(), undefined],
  ["output_devices", (b) => b.outputDevices(), undefined],
  ["output_select", (b) => b.outputSelect("dev-1"), { deviceId: "dev-1" }],
  ["output_select", (b) => b.outputSelect(null), { deviceId: null }],
  ["health", (b) => b.health(), undefined],
  ["cached_tracks", (b) => b.cachedTracks(), undefined],
  ["settings_get", (b) => b.settingsGet(), undefined],
  ["settings_set", (b) => b.settingsSet({ privateSession: true }), { patch: { privateSession: true } }],
  ["art_cache_info", (b) => b.artCacheInfo(), undefined],
  ["art_cache_clear", (b) => b.artCacheClear(), undefined],
  ["app_info", (b) => b.appInfo(), undefined],
  ["open_project_page", (b) => b.openProjectPage(), undefined],
  ["update_check", (b) => b.updateCheck(), undefined],
  ["update_install_on_restart", (b) => b.updateInstallOnRestart(), undefined],
  ["eq_profile_save", (b) => b.eqProfileSave("Desk"), { name: "Desk" }],
  ["eq_profile_rename", (b) => b.eqProfileRename("Desk", "Studio"), { from: "Desk", to: "Studio" }],
  ["eq_profile_delete", (b) => b.eqProfileDelete("Studio"), { name: "Studio" }],
  ["eq_use_for_output", (b) => b.eqUseForOutput(true), { on: true }],
  ["headphone_search", (b) => b.headphoneSearch("hd 600"), { query: "hd 600" }],
  ["headphone_correction_set", (b) => b.headphoneCorrectionSet(null), { path: null }],
  ["headphone_correction_compare", (b) => b.headphoneCorrectionCompare(true), { bypass: true }],
  ["bit_perfect_set", (b) => b.bitPerfectSet(true), { on: true }],
  ["plex_edit_access", (b) => b.plexEditAccess(), undefined],
  ["plex_edit_item", (b) => b.plexEditItem("a:5"), { id: "a:5" }],
  ["plex_edit_images", (b) => b.plexEditImages("5", "art"), { id: "5", which: "art" }],
  ["plex_edit_select_image", (b) => b.plexEditSelectImage("5", "album", "poster", "k"), { id: "5", kind: "album", which: "poster", key: "k" }],
  [
    "plex_edit_upload_image",
    (b) => b.plexEditUploadImage("5", "album", "poster", new Uint8Array([1, 2]), "image/png"),
    { id: "5", kind: "album", which: "poster", bytes: [1, 2], contentType: "image/png" },
  ],
  ["plex_edit_image_link", (b) => b.plexEditImageLink("5", "artist", "art", "https://x"), { id: "5", kind: "artist", which: "art", url: "https://x" }],
  ["plex_edit_matches", (b) => b.plexEditMatches("5", "T", 2020), { id: "5", title: "T", year: 2020 }],
  [
    "plex_edit_match",
    (b) => b.plexEditMatch("5", "album", { guid: "g", name: "N", artist: null, year: null, score: 9 }),
    { id: "5", kind: "album", guid: "g", name: "N", year: null },
  ],
  ["plex_edit_unmatch", (b) => b.plexEditUnmatch("5", "album"), { id: "5", kind: "album" }],
  ["plex_edit_merge", (b) => b.plexEditMerge("7", ["8"], "8"), { target: "7", others: ["8"], photoFrom: "8" }],
  ["plex_edit_merge_albums", (b) => b.plexEditMergeAlbums("7", ["8"]), { target: "7", others: ["8"] }],
  ["doctor_report", (b) => b.doctorReport(), undefined],
  ["doctor_count", (b) => b.doctorCount(), undefined],
  ["doctor_ignore", (b) => b.doctorIgnore("cover:1", "cover", "No cover", true), { id: "cover:1", kind: "cover", title: "No cover", ignore: true }],
  ["doctor_run", (b) => b.doctorRun(), undefined],
  ["plex_edit_split", (b) => b.plexEditSplit("7"), { id: "7" }],
  ["plex_edit_refresh", (b) => b.plexEditRefresh("7", "artist"), { id: "7", kind: "artist" }],
  ["plex_edit_undo", (b) => b.plexEditUndo("c1"), { changeId: "c1" }],
  ["plex_edit_history", (b) => b.plexEditHistory("5"), { id: "5" }],
];

const EVENTS = [
  "auth://changed",
  "sync://progress",
  "library://changed",
  "artist://changed",
  "player://state",
  "player://position",
  "player://error",
  "output://changed",
  "output://lost",
  "health://changed",
  "toast",
  "update://ready",
  "settings://changed",
] as const;

describe("Tauri bridge matches docs/IPC.md", () => {
  beforeEach(() => {
    invoke.mockClear();
    listen.mockClear();
  });

  it.each(CALLS)("%s", async (cmd, call, args) => {
    await call(createTauriBackend());
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke.mock.calls[0][0]).toBe(cmd);
    if (args) expect(invoke.mock.calls[0][1]).toEqual(args);
  });

  it("subscribes to every event by its exact name", async () => {
    const b = createTauriBackend();
    for (const e of EVENTS) await b.listen(e, () => {});
    expect(listen.mock.calls.map((c) => c[0])).toEqual([...EVENTS]);
  });

  it("normalizes thrown errors to AppError", async () => {
    invoke.mockRejectedValueOnce("boom");
    await expect(createTauriBackend().home()).rejects.toEqual({ kind: "internal", message: "boom" });
    invoke.mockRejectedValueOnce({ kind: "network", message: "offline" });
    await expect(createTauriBackend().home()).rejects.toEqual({ kind: "network", message: "offline" });
  });

  it("builds art URLs through the art scheme at a pixel size", () => {
    const url = createTauriBackend().artUrl("/library/metadata/1/thumb/2", 200, 200);
    expect(url).toBe(`http://art.localhost/${encodeURIComponent("/library/metadata/1/thumb/2")}?w=200&h=200`);
  });
});

describe("art sizes", () => {
  it("requests rendered size × pixel ratio, rounded up to a shared step", () => {
    expect(artPixelSize(40, 1)).toBe(40);
    expect(artPixelSize(41, 1)).toBe(48);
    expect(artPixelSize(40, 1.5)).toBe(64);
    expect(artPixelSize(56, 1.25)).toBe(80);
    expect(artPixelSize(232, 1)).toBe(240);
    expect(artPixelSize(232, 2)).toBe(480);
    expect(artPixelSize(232, 1.5)).toBe(400);
    expect(artPixelSize(176, 1.5)).toBe(280);
    expect(artBucket(2000)).toBe(2560);
    expect(artBucket(99999)).toBe(4096);
  });

  it("reads the display's pixel ratio, clamped", () => {
    expect(devicePixelRatio({ devicePixelRatio: 1.5 })).toBe(1.5);
    expect(devicePixelRatio({ devicePixelRatio: 0 })).toBe(1);
    expect(devicePixelRatio({ devicePixelRatio: 9 })).toBe(4);
    expect(devicePixelRatio({})).toBe(1);
  });

  it("uses the display's pixel ratio for UI art URLs", () => {
    const prev = window.devicePixelRatio;
    Object.defineProperty(window, "devicePixelRatio", { value: 1, configurable: true });
    const spy = vi.spyOn(backend, "artUrl");
    artUrl("/a", 56);
    expect(spy).toHaveBeenLastCalledWith("/a", 64, 64);
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    artUrl("/a", 56);
    expect(spy).toHaveBeenLastCalledWith("/a", 128, 128);
    spy.mockRestore();
    Object.defineProperty(window, "devicePixelRatio", { value: prev, configurable: true });
  });
});

describe("mock backend covers the whole contract", () => {
  it("implements every command the Tauri bridge does", () => {
    const tauriKeys = Object.keys(createTauriBackend()).filter((k) => k !== "kind").sort();
    const m = new MockBackend({ timelines: false, latencyMs: 0 });
    for (const k of tauriKeys) expect(typeof (m as unknown as Record<string, unknown>)[k], k).toBe("function");
    m.dispose();
  });

  it("emits every event type", async () => {
    vi.useFakeTimers();
    const m = new MockBackend({ timelines: false, latencyMs: 0 });
    const seen = new Set<string>();
    for (const e of EVENTS) await m.listen(e, () => seen.add(e));
    m.emit("update://ready", { version: "0.2.0" });
    m.emit("library://changed", null);
    m.emit("artist://changed", { id: m.fx.artists[0].id });
    m.emit("sync://progress", { phase: "idle", done: 0, total: 0 });
    m.setHealth("offline");
    m.setHealth("local"); // server back → toast
    await m.authStart();
    await m.playContext({ context: { kind: "album", id: m.fx.albums[0].id } });
    // A corrupt track in an untagged build triggers player://error.
    const broken = m.fx.tracks.find((t) => t.title.endsWith("(corrupt rip)"))!;
    await m.playContext({ context: { kind: "album", id: broken.albumId }, startTrackId: broken.id });
    await m.playResume();
    vi.advanceTimersByTime(300);
    // Choosing a device, then unplugging it: output://changed and output://lost.
    await m.outputSelect("mock:headphones");
    // An EQ preset bound to it: unplugging goes back to the earlier choice (settings://changed).
    await m.settingsSet({ eqPreset: "vocal" });
    await m.eqUseForOutput(true);
    await m.settingsSet({ eqPreset: "bassBoost" });
    m.unplugOutput("mock:headphones");
    vi.useRealTimers();
    expect([...seen].sort()).toEqual([...EVENTS].sort());
    m.dispose();
  });

  it("has a realistic invented library", () => {
    const m = new MockBackend({ timelines: false, latencyMs: 0 });
    // Twelve generated artists, plus the archive demo artist.
    expect(m.fx.artists).toHaveLength(13);
    expect(m.fx.albums.length).toBeGreaterThanOrEqual(55);
    expect(m.fx.tracks.length).toBeGreaterThanOrEqual(500);
    const sections = new Set(m.fx.albums.map((a) => a.section));
    for (const s of ["albums", "singlesEps", "live", "soundtracks", "compilations", "remixes", "demos"]) expect(sections.has(s as never)).toBe(true);
    expect(m.fx.albums.some((a) => a.format === null)).toBe(true);
    const codecs = new Set(m.fx.tracks.map((t) => t.codec));
    expect([...codecs].sort()).toEqual(["aac", "flac", "mp3"]);
    m.dispose();
  });
});
