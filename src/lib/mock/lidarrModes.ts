import type {
  AppError,
  LidarrArtistMode,
  LidarrArtistPrefs,
  LidarrCatchUpReport,
  LidarrChange,
  LidarrFixAction,
  LidarrHuntItem,
  LidarrModeSetting,
  LidarrProposal,
  LidarrRematchPlan,
  LidarrStaged,
  LidarrTarget,
  LidarrWork,
} from "../types";

const invalid = (message: string): AppError => ({ kind: "invalid", message });

const QH = "b0000000-0000-0000-0000-000000000002";
const GH = "c0000000-0000-0000-0000-000000000003";

/** The demo library's artists and their modes. */
function demoArtists(): LidarrArtistMode[] {
  const base = { mbid: null, lidarrId: null, lidarrPath: null, folder: null, setting: "auto" as const, files: 0, curatedFiles: 0, excluded: false, reasons: [] };
  return [
    {
      ...base,
      key: QH,
      name: "Quiet Hours",
      artistId: "ar-qh",
      mbid: QH,
      lidarrId: 7,
      lidarrPath: "/data/staging/music/Quiet Hours",
      folder: "/data/media/Music/Archive/Quiet Hours",
      mode: "protected",
      auto: "protected",
      line: "Protected: 403 files tagged unreleased or leaked",
      reasons: ["403 files tagged unreleased or leaked"],
      files: 512,
      curatedFiles: 403,
    },
    {
      ...base,
      key: GH,
      name: "Glasshouse",
      artistId: "ar-gh",
      mbid: GH,
      mode: "protected",
      auto: "protected",
      line: "Protected: on Lidarr's import list exclusions",
      reasons: ["on Lidarr's import list exclusions"],
      files: 40,
      excluded: true,
    },
    {
      ...base,
      key: "a0000000-0000-0000-0000-000000000001",
      name: "Northlight",
      artistId: "ar-nl",
      lidarrId: 9,
      lidarrPath: "/data/media/Music/Lidarr/Northlight",
      mode: "managed",
      auto: "managed",
      line: "Managed: all files official, no curated tags",
      files: 36,
    },
    {
      ...base,
      key: "d0000000-0000-0000-0000-000000000004",
      name: "Harbor Sound",
      artistId: null,
      lidarrId: 12,
      lidarrPath: "/data/media/Music/Lidarr/Harbor Sound",
      mode: "managed",
      auto: "managed",
      line: "Managed: in Lidarr only, nothing of it in your library yet",
    },
  ];
}

function demoStaged(now: number): LidarrStaged[] {
  return [
    {
      id: "srv:30",
      albumId: 30,
      lidarrArtistId: 7,
      artist: "Quiet Hours",
      title: "Third Way",
      releaseGroup: "30000000-0000-0000-0000-000000000003",
      partial: false,
      state: "ready",
      reason: "Waiting for you: the first new album for Quiet Hours is always shown first.",
      plan: {
        folder: "Third Way (2026)",
        files: [
          {
            src: "/data/staging/music/Quiet Hours/Third Way (2026)/01 - Arrive.flac",
            name: "1-01 - Arrive.flac",
            edits: [
              ["ALBUMARTIST", "Quiet Hours"],
              ["GENRE", "Ambient"],
              ["DISCNUMBER", "1"],
              ["STATUS", "Released"],
              ["VERSION", "Official release (Lidarr, FLAC, 2026-10-16)"],
            ],
          },
          {
            src: "/data/staging/music/Quiet Hours/Third Way (2026)/02 - Leave.flac",
            name: "1-02 - Leave.flac",
            edits: [
              ["ALBUMARTIST", "Quiet Hours"],
              ["STATUS", "Released"],
              ["VERSION", "Official release (Lidarr, FLAC, 2026-10-16)"],
            ],
          },
        ],
      },
      folder: "/data/media/Music/Archive/Quiet Hours/Third Way (2026)",
      confident: true,
      doubts: [],
      auto: false,
      at: now - 3600,
      updatedAt: now - 3600,
    },
    {
      id: "srv:31",
      albumId: 31,
      lidarrArtistId: 7,
      artist: "Quiet Hours",
      title: "Half Done",
      releaseGroup: "30000000-0000-0000-0000-000000000004",
      partial: true,
      state: "waiting",
      reason: "Waiting for you: Lidarr imported only part of this album. Platter never adds a partial album.",
      plan: null,
      folder: null,
      confident: false,
      doubts: [],
      auto: false,
      at: now - 7200,
      updatedAt: now - 7200,
    },
  ];
}

/**
 * Managed and protected artists in the browser mock: modes, choices, fixes
 * that ask first, the changes log, albums waiting in staging and practice
 * mode.
 */
export class MockLidarrWork {
  artists = demoArtists();
  prefs: Record<string, LidarrArtistPrefs> = {};
  changes: LidarrChange[] = [];
  staged: LidarrStaged[];
  proposals = new Map<string, LidarrProposal>();
  practiceUntil: number | null;
  paused: string | null = null;
  private n = 0;

  constructor(
    private now: () => number,
    private practiceSetting: () => number,
  ) {
    this.staged = demoStaged(now());
    this.practiceUntil = now() + 5 * 86_400;
  }

  /** Back to the demo state (tests). */
  reset() {
    this.artists = demoArtists();
    this.prefs = {};
    this.changes = [];
    this.staged = demoStaged(this.now());
    this.proposals.clear();
    this.practiceUntil = this.now() + 5 * 86_400;
    this.paused = null;
  }

  private log(what: string, before: string, after: string, asked: boolean, undo: unknown = null, practice = false): LidarrChange {
    const e: LidarrChange = { id: `c${++this.n}`, at: this.now(), device: "This computer", what, before, after, asked, undo, putBackAt: null, practice };
    this.changes.unshift(e);
    return e;
  }

  modes(): LidarrArtistMode[] {
    return this.artists.map((a) => {
      const setting = this.prefs[a.key]?.mode ?? "auto";
      const mode = setting === "auto" ? a.auto : setting;
      const line =
        setting === "managed" && a.auto === "protected"
          ? `Managed: you chose Managed (Auto would protect it: ${a.reasons[0] ?? ""})`
          : setting === "protected" && a.auto === "managed"
            ? "Protected: you chose Protected"
            : a.line;
      return { ...a, setting, mode, line };
    });
  }

  setMode(key: string, name: string, mode: LidarrModeSetting, confirmations: number): LidarrArtistPrefs {
    if (mode === "managed" && confirmations < 2) {
      throw invalid("Managed lets Lidarr rename, retag and replace this artist's files. Confirm twice to switch.");
    }
    const p = { ...this.choicesFor(key), mode, name };
    this.prefs[key] = p;
    this.log(`Set ${name} to ${mode === "auto" ? "Auto" : mode === "managed" ? "Managed" : "Protected"}`, "", "", true);
    return p;
  }

  choicesFor(key: string): LidarrArtistPrefs {
    return this.prefs[key] ?? { mode: "auto", autoAdd: null, hunt: false, target: "lossless", name: "" };
  }

  setChoices(key: string, name: string, autoAdd: boolean | null, hunt: boolean, target: LidarrTarget): LidarrArtistPrefs {
    const p = { ...this.choicesFor(key), autoAdd, hunt, target, name };
    this.prefs[key] = p;
    return p;
  }

  propose(action: LidarrFixAction, input: string | null): LidarrProposal {
    let what = "";
    let before = "";
    let after = "";
    switch (action.kind) {
      case "addExclusions": {
        const names = action.artists.map((a) => a.name).join(", ");
        what = "Add protected artists to Lidarr's import list exclusions";
        before = `Not on the list: ${names}`;
        after = `On the list: ${names}. Lidarr's import lists never add them.`;
        break;
      }
      case "setRecycleBin": {
        const path = (input ?? "").trim().replace(/[\\/]+$/, "");
        if (!path) throw invalid("Write the recycling bin folder as Lidarr sees it, like /data/lidarr-recycle.");
        if (/^\/data\/media\/music(\/|$)/i.test(path)) throw invalid(`${path} is inside your Plex music folder /data/media/Music, so deleted files would show up in Plex. Pick a folder outside it.`);
        what = "Set Lidarr's recycling bin";
        before = "(none)";
        after = path;
        break;
      }
      case "writeTagsOff":
        what = "Turn off Write Audio Tags";
        before = "newFiles";
        after = "no";
        break;
      case "renameOff":
        what = "Turn off Rename Tracks";
        before = "On";
        after = "Off";
        break;
      case "addStagingRoot":
        what = "Add the staging folder to Lidarr";
        before = "Not a root folder";
        after = `Root folder ${action.path}, monitoring nothing by default`;
        break;
      case "rematch":
        throw invalid("Re-match an album from its own row.");
    }
    const p: LidarrProposal = { id: `p${++this.n}`, what, before, after, canPutBack: true };
    this.proposals.set(p.id, p);
    return p;
  }

  hiresPreview(): LidarrProposal {
    const p: LidarrProposal = {
      id: `p${++this.n}`,
      what: "Add the Platter Hi-Res quality profile",
      before: "No hi-res profile",
      after: "Platter Hi-Res: 24-bit lossless only, upgrades until FLAC 24bit",
      canPutBack: true,
    };
    this.proposals.set(p.id, p);
    return p;
  }

  apply(id: string): LidarrChange {
    const p = this.proposals.get(id);
    if (!p) throw invalid("That change was shown a while ago. Look at it again first.");
    this.proposals.delete(id);
    return this.log(p.what, p.before, p.after, true, { kind: "demo" });
  }

  putBack(id: string): LidarrChange {
    const e = this.changes.find((c) => c.id === id);
    if (!e) throw { kind: "notFound", message: "That change isn't in the log." } satisfies AppError;
    if (e.putBackAt) throw invalid("That change was already put back.");
    if (!e.undo) throw invalid("This change can't be put back from Platter.");
    e.putBackAt = this.now();
    return { ...e };
  }

  rematch(folder: string): LidarrRematchPlan {
    return {
      folder,
      albumId: 10,
      album: "First Light",
      release: "CD",
      files: [`${folder}/1-02 Noon.flac`],
      kept: [{ path: `${folder}/3-01 Bonus.flac`, why: "not on the album's current edition (a bonus track); kept as it is" }],
      otherEdition: 0,
      linkedNow: 11,
      editionNote: null,
    };
  }

  work(): LidarrWork {
    const setting = this.practiceSetting();
    const until = setting === -1 ? this.practiceUntil : setting === 0 ? null : setting;
    return {
      catchUp: { lastDevice: "This computer", lastAt: this.now() - 240, gap: null, paused: this.paused },
      practiceUntil: until,
      practice: until !== null && until > this.now(),
      staged: this.staged.map((s) => ({ ...s })),
    };
  }

  catchUp(): LidarrCatchUpReport {
    return { handled: 0, elsewhere: 0, settling: 0, acted: 0, gap: null, scanned: [], announced: [], staged: [], healed: [], paused: this.paused };
  }

  resume() {
    this.paused = null;
  }

  stagedAdd(id: string, always: boolean): LidarrStaged {
    const s = this.staged.find((x) => x.id === id);
    if (!s) throw { kind: "notFound", message: "That album isn't waiting any more." } satisfies AppError;
    if (s.partial) throw invalid(s.reason ?? "Waiting for you.");
    s.state = "added";
    s.reason = null;
    s.updatedAt = this.now();
    this.log(`Added ${s.title} by ${s.artist} to your library`, "In the staging folder", `${s.folder} (${s.plan?.files.length ?? 0} files)`, true);
    this.log(`Stopped Lidarr looking for ${s.title}`, "Monitored", "Not monitored", false, { kind: "monitor" });
    this.log(`Freed ${s.title}'s copy in the staging folder`, "2 files in staging", "In Lidarr's recycling bin (/data/lidarr-recycle); the space comes back after Lidarr's recycling bin cleanup", false);
    if (always) {
      const a = this.artists.find((x) => x.lidarrId === s.lidarrArtistId);
      if (a) this.prefs[a.key] = { ...this.choicesFor(a.key), autoAdd: true, name: a.name };
    }
    return { ...s };
  }

  stagedUndo(id: string, trashOk: boolean): LidarrStaged {
    const s = this.staged.find((x) => x.id === id);
    if (!s || s.state !== "added") throw invalid("Only an added album can be taken out again.");
    if (!trashOk) {
      throw invalid(
        "Plex empties its trash after every scan, which would drop this album's play counts and ratings from Plex during the move. Platter can turn that off just for the move and back on right after.",
      );
    }
    s.state = "undone";
    s.updatedAt = this.now();
    this.log(`Took ${s.title} out of your library`, s.folder ?? "", "_to_delete", true);
    return { ...s };
  }

  hunt(): LidarrHuntItem[] {
    return [{ albumId: "q1", title: "Sessions", artist: "Quiet Hours", now: "MP3 320", state: "requested", note: null }];
  }
}
