/** Settings page group for editing in Plex. All three travel with the sync file. */
import { useId } from "react";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { updateSettings, useSettings } from "../stores/settings";
import { useCanEdit, usePlexEdit } from "../stores/plexEdit";
import type { Settings } from "../lib/types";

type Key = "plexEditShow" | "plexEditLock" | "plexEditMatchUnreleased";

export const PLEX_EDIT_ROWS: { key: Key; title: string; description: string }[] = [
  {
    key: "plexEditShow",
    title: "Show editing options",
    description:
      "Adds Edit in Plex to album and artist menus: change the cover or photo, fix the match, edit details, merge and split artists. Only on servers you own or administer, and only Plex's own data changes, never your files.",
  },
  {
    key: "plexEditLock",
    title: "Lock fields I edit",
    description: "Fields you change in Edit details are locked in Plex, so refreshing the album or artist keeps your changes.",
  },
  {
    key: "plexEditMatchUnreleased",
    title: "Fix match on unreleased albums",
    description:
      "Shows Fix match on albums that count as unreleased, always with a warning that a match can replace their details with a different release. Off hides it on them. Unmatch stays either way.",
  },
];

/** Whether the search box matches any of these rows (or the group). */
export function plexEditMatches(query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return "edit in plex".includes(q) || PLEX_EDIT_ROWS.some((r) => `${r.title} ${r.description}`.toLowerCase().includes(q));
}

function Row({ k, title, description, s, disabled }: { k: Key; title: string; description: string; s: Settings | null; disabled?: boolean }) {
  const id = useId();
  return (
    <div className="setting-row">
      <div className="setting-row__text">
        <div id={`${id}-t`} className="setting-row__title">
          {title}
        </div>
        <p id={`${id}-d`} className="setting-row__desc">
          {description}
        </p>
      </div>
      {s ? (
        <Switch checked={s[k] !== false} disabled={disabled} onChange={(v) => updateSettings({ [k]: v } as Partial<Settings>)} labelledBy={`${id}-t`} describedBy={`${id}-d`} />
      ) : (
        <Skeleton width={40} height={24} radius="pill" />
      )}
    </div>
  );
}

export function PlexEditSettings() {
  const s = useSettings((x) => x.settings);
  // Loads which servers can be edited; the note says when none can.
  useCanEdit(null);
  const access = usePlexEdit((x) => x.access);
  const none = !!access && access.servers.length === 0 && !access.connected;
  const off = s?.plexEditShow === false;
  return (
    <section className="settings__group" aria-labelledby="settings-edit-in-plex">
      <h2 id="settings-edit-in-plex" className="settings__heading">
        Edit in Plex
      </h2>
      {none && (
        <p className="setting-row__desc" role="note">
          Your account doesn't own any of its servers, so there's nothing to edit. The server's owner can edit from their Platter.
        </p>
      )}
      {PLEX_EDIT_ROWS.map((r) => (
        <Row key={r.key} k={r.key} title={r.title} description={r.description} s={s} disabled={r.key !== "plexEditShow" && off} />
      ))}
    </section>
  );
}
