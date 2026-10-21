/**
 * Mock sharing through Plex: your server's music libraries, the people with
 * access, invites you received, and share links. Invented people only.
 *
 * Test hooks: `plexPass`, `owner`, `failNext` ("alreadyShared",
 * "unknownUser", "refused", "plexDown"), `inviteArrives(from)`,
 * `labelled` (albums marked to hide), `shared` (every share made).
 */
import type { InviteAccepted, PersonWithAccess, ReceivedInvite, Settings, ShareLinkResult, ShareLinkView, ShareOptions, ShareStatus, SharingOverview } from "../types";
import { mockQr } from "./sync";

export interface SharingHooks {
  settings: () => Settings;
  emitInvites: (all: ReceivedInvite[]) => void;
  emitNotice: (message: string) => void;
  later: (ms: number, fn: () => void) => void;
  /** An invite was accepted: the host's server and its music are in the library now. */
  accepted: (from: string) => void;
}

export const MOCK_SHARE_LINK = "platter://share/AV8MKp57HUw4puL5CxfT5EXMOOHwS3stlqDkyLHT9XopaPBRgA";

/** The music libraries on the mock server. Films and photos are never offered. */
const LIBRARIES = [
  { id: "3", title: "Music", unreleased: 4 },
  { id: "7", title: "Live Tapes", unreleased: 0 },
  { id: "9", title: "Old Rips", unreleased: null as number | null },
];

const DAY = 86400;

export class MockSharing {
  plexPass = true;
  owner = true;
  failNext: "alreadyShared" | "unknownUser" | "refused" | "plexDown" | null = null;
  /** Every share made: who, and what it allowed. */
  shared: { invitee: string; options: ShareOptions; excludesLabel: boolean }[] = [];
  /** Albums marked so shares leave them out. */
  labelled = 0;
  private people: PersonWithAccess[] = [];
  private invites: ReceivedInvite[] = [];
  private nextId = 5001;

  constructor(private hooks: SharingHooks) {
    this.reset();
  }

  reset() {
    const now = Math.floor(Date.now() / 1000);
    this.plexPass = true;
    this.owner = true;
    this.failNext = null;
    this.shared = [];
    this.labelled = 0;
    this.invites = [];
    this.people = [
      { id: "4001", userId: "71", name: "robin_listens", libraries: ["Music"], allowDownloads: false, unreleasedHidden: true, pending: false, lastSeenAt: now - 2 * 3600 },
      { id: "4002", userId: "72", name: "jules@example.com", libraries: ["Live Tapes"], allowDownloads: true, unreleasedHidden: false, pending: true, lastSeenAt: null },
    ];
  }

  overview(): SharingOverview {
    if (!this.owner) return { serverName: "Morgan's Music", owner: false, plexPass: null, libraries: [] };
    const defaults = this.hooks.settings().shareDefaultLibraries;
    return {
      serverName: "Basement Server",
      owner: true,
      plexPass: this.plexPass,
      libraries: LIBRARIES.map((l) => ({ ...l, default: defaults.includes(l.id) })),
    };
  }

  share(invitee: string, options: ShareOptions): ShareStatus {
    if (!this.owner) throw { kind: "invalid", message: "Only the server's owner can share it. This server is shared with you." };
    const who = invitee.trim();
    if (!who) throw { kind: "invalid", message: "Type their Plex username or email." };
    const libs = [...new Set(options.libraries)];
    if (!libs.length) throw { kind: "invalid", message: "Pick a music library to share." };
    if (libs.some((id) => !LIBRARIES.some((l) => l.id === id))) throw { kind: "invalid", message: "Only music libraries can be shared from Platter." };
    const chosen = LIBRARIES.filter((l) => libs.includes(l.id));
    const fail = this.failNext;
    this.failNext = null;
    if (fail === "plexDown") return { status: "plexDown", message: "Plex isn't answering right now. Try again in a while." };
    let hidden = 0;
    if (options.hideUnreleased) {
      const withUnreleased = chosen.filter((l) => l.unreleased !== 0);
      if (withUnreleased.length && !this.plexPass) {
        return { status: "needsPlexPass", withUnreleased: withUnreleased.map((l) => l.title), others: chosen.filter((l) => l.unreleased === 0).map((l) => l.id) };
      }
      const unread = withUnreleased.find((l) => l.unreleased === null);
      if (unread) {
        return { status: "refused", message: `Platter hasn't read ${unread.title} yet, so it can't keep its unreleased music out. Include it in Settings, Library, then share.` };
      }
      hidden = withUnreleased.reduce((n, l) => n + (l.unreleased ?? 0), 0);
      this.labelled = Math.max(this.labelled, hidden);
    }
    if (fail === "alreadyShared") return { status: "alreadyShared" };
    if (fail === "unknownUser") return { status: "unknownUser", message: "Plex doesn't know that username or email." };
    if (fail === "refused") return { status: "refused", message: "Plex turned the invite down." };
    this.shared.push({ invitee: who, options: { ...options, libraries: libs }, excludesLabel: options.hideUnreleased && this.plexPass });
    this.people = [
      ...this.people.filter((p) => p.name !== who),
      {
        id: String(this.nextId),
        userId: String(this.nextId++ + 1000),
        name: who,
        libraries: chosen.map((l) => l.title),
        allowDownloads: options.allowDownloads,
        unreleasedHidden: options.hideUnreleased && this.plexPass,
        pending: true,
        lastSeenAt: null,
      },
    ];
    return { status: "invited", hidden };
  }

  peopleList(): PersonWithAccess[] {
    if (!this.owner) throw { kind: "invalid", message: "Only the server's owner can share it. This server is shared with you." };
    return this.people.map((p) => ({ ...p, libraries: [...p.libraries] }));
  }

  remove(id: string) {
    if (!this.people.some((p) => p.id === id)) throw { kind: "notFound", message: "They don't have access anymore." };
    this.people = this.people.filter((p) => p.id !== id);
  }

  link(options: ShareOptions): ShareLinkView {
    if (!options.libraries.length) throw { kind: "invalid", message: "Pick a music library to share." };
    const link = MOCK_SHARE_LINK;
    return { link, qr: mockQr(link), expiresAt: Date.now() + this.hooks.settings().listenInviteHours * 3600 * 1000 };
  }

  openLink(link: string): ShareLinkResult {
    if (!link.startsWith("platter://share/")) throw { kind: "invalid", message: "That isn't a share link." };
    this.hooks.later(800, () => this.inviteArrives("Morgan"));
    return { status: "invited", host: "Morgan" };
  }

  invitesList(): ReceivedInvite[] {
    return this.invites.map((i) => ({ ...i, servers: [...i.servers] }));
  }

  /** A friend shares their server with you (test hook). */
  inviteArrives(from: string) {
    if (this.invites.some((i) => i.from === from)) return;
    this.invites = [...this.invites, { id: String(8000 + this.invites.length + 1), from, servers: [`${from}'s Music`], friend: true, home: false, server: true, createdAt: Math.floor(Date.now() / 1000) - DAY / 24 }];
    this.hooks.emitInvites(this.invitesList());
  }

  accept(id: string): InviteAccepted {
    const inv = this.invites.find((i) => i.id === id);
    if (!inv) throw { kind: "notFound", message: "That invite is gone. It may have been taken back." };
    this.invites = this.invites.filter((i) => i.id !== id);
    this.hooks.emitInvites(this.invitesList());
    this.hooks.accepted(inv.from);
    return { servers: [...inv.servers], libraries: 1 };
  }
}
