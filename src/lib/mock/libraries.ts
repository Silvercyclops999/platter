import type { AppError, LibraryServers, LibrarySources, SourceServer } from "../types";

/**
 * The account's servers for Settings, Library and the sidebar switcher: the
 * connected one with two libraries, an owned one that's offline, and a
 * friend's shared server.
 */
const SERVERS: SourceServer[] = [
  {
    machineId: "mock-machine-01",
    tag: "1a2b3c4d",
    name: "Basement Server",
    owned: true,
    current: true,
    online: true,
    connection: "local",
    libraries: [
      { id: "1", title: "Music", included: true, main: true },
      { id: "4", title: "Archive", included: false, main: false },
    ],
  },
  {
    machineId: "mock-machine-02",
    tag: "5e6f7a8b",
    name: "Cabin NAS",
    owned: true,
    current: false,
    online: false,
    connection: null,
    libraries: [],
  },
  {
    machineId: "mock-friend-03",
    tag: "9c0d1e2f",
    name: "Zed's Plex",
    owned: false,
    current: false,
    online: true,
    connection: "relay",
    libraries: [{ id: "7", title: "Zed's music", included: false, main: false }],
  },
];

const entry = (s: SourceServer, id: string) => `${s.machineId}/${id}`;

export class MockLibraries {
  /** Test hook: list only the connected server. */
  oneServer = false;

  sources(included: string[]): LibrarySources {
    const servers = (this.oneServer ? SERVERS.slice(0, 1) : SERVERS).map((s) => ({
      ...s,
      libraries: s.libraries.map((l) => ({ ...l, included: l.main || included.includes(entry(s, l.id)) })),
    }));
    return { servers };
  }

  servers(included: string[], view: string): LibraryServers {
    const list = SERVERS.filter((s) => s.current || included.some((e) => e.startsWith(`${s.machineId}/`))).map((s) => ({
      tag: s.tag,
      name: s.name,
    }));
    const known = list.length > 1 && list.some((s) => s.tag === view);
    return { servers: list, view: known ? view : "all" };
  }

  /** Same checks as the core for `libraries` and `serverView`. */
  check(libraries: string[] | undefined, view: string | undefined): void {
    if (libraries?.some((e) => !/^[A-Za-z0-9_-]{1,100}\/\d{1,10}$/.test(e.trim()))) {
      throw { kind: "invalid", message: "That isn't a music library on your servers." } satisfies AppError;
    }
    if (view !== undefined && view !== "all" && !/^[0-9a-f]{8}$/.test(view)) {
      throw { kind: "invalid", message: "Choose All servers or one of your servers." } satisfies AppError;
    }
  }
}
