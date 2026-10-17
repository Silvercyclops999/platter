/**
 * Devices: the drives and players music is synced to. Each device shows
 * whether it's plugged in and its free space, what goes on it (in the order
 * it was chosen), its format, size limit and music folder, and the sync
 * with its progress, pause and cancel, and what didn't fit or failed.
 * Removable drives that aren't a device yet are offered at the top.
 */
import { Disc3, HardDrive, Heart, ListMusic, MoreHorizontal, Pause, Play, RefreshCw, User, X } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { Button, IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { AddDeviceDialog, ForgetDeviceDialog, RenameDeviceDialog, driveName } from "../components/DeviceDialogs";
import { Select, type SelectOption } from "../components/Select";
import { SkeletonLines } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { formatBytes } from "../lib/format";
import { timeAgo } from "../lib/relativeTime";
import type { AppError, DeviceFormat, DeviceItem, DeviceItemKind, DeviceSyncStatus, DeviceView, DriveVolume } from "../lib/types";
import { useApp } from "../stores/app";
import { FORMAT_LABELS, FORMAT_ORDER, devices, loadDevices, progressLine, running, startDevices, useDevices } from "../stores/devices";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./devices.css";

/** How often the page looks for drives plugged in or out while it's open. */
export const DRIVE_POLL_MS = 5000;
const BUDGETS = [2, 4, 8, 16, 32, 64, 128, 256, 512];

const KIND_LABEL: Record<DeviceItemKind, string> = { playlist: "Playlist", album: "Album", artist: "Artist", liked: "Playlist" };
const KIND_ICON = { playlist: ListMusic, album: Disc3, artist: User, liked: Heart } as const;

/** "21.4 GB free of 29.7 GB". */
export function spaceLine(free: number | null, total: number | null): string | null {
  if (free === null) return null;
  return total ? `${formatBytes(free)} free of ${formatBytes(total)}` : `${formatBytes(free)} free`;
}

export function DevicesPage() {
  const view = useDevices((s) => s.view);
  const error = useDevices((s) => s.error);
  const [adding, setAdding] = useState<DriveVolume | null>(null);
  useEffect(() => {
    void startDevices().then(() => loadDevices());
    const t = setInterval(() => void loadDevices(), DRIVE_POLL_MS);
    return () => clearInterval(t);
  }, []);

  const list = view?.devices ?? [];
  const drives = view?.drives ?? [];
  return (
    <div className="page-pad devices" data-testid="devices-page">
      <header className="devices__head">
        <h1 className="page-title">Devices</h1>
        <p className="devices__sub">
          Copy playlists, albums, artists and Liked Songs to a USB drive, an SD card or a music player. Platter only changes files it put there.
        </p>
      </header>

      {error && !view && (
        <p className="devices__empty" role="alert">
          {error}
        </p>
      )}
      {!view && !error && <SkeletonLines lines={4} widths={["70%", "90%", "80%", "60%"]} gap={24} />}

      {drives.length > 0 && (
        <section className="devices__drives" aria-label="Plugged in">
          <h2 className="devices__section-title">Plugged in</h2>
          <ul className="devices__list">
            {drives.map((v) => (
              <li key={v.root} className="drive-row">
                <span className="drive-row__icon" aria-hidden="true">
                  <HardDrive size={20} />
                </span>
                <div className="drive-row__body">
                  <span className="drive-row__name">{driveName(v)}</span>
                  <span className="drive-row__meta">{spaceLine(v.free, v.total) ?? "Free space unknown"}</span>
                </div>
                <Button size="sm" variant="primary" onClick={() => setAdding(v)}>
                  Add as a device
                </Button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {view && list.length === 0 && drives.length === 0 && (
        <div className="devices__empty-state" role="status">
          <HardDrive size={28} aria-hidden="true" />
          <p>Plug in a USB drive, an SD card or a music player to get started. It shows up here.</p>
        </div>
      )}

      {list.map((d) => (
        <DeviceCard key={d.id} d={d} sync={view?.sync?.deviceId === d.id ? view.sync : null} busy={running(view?.sync)} />
      ))}

      {adding && <AddDeviceDialog drive={adding} onClose={() => setAdding(null)} />}
    </div>
  );
}

function DeviceCard({ d, sync, busy }: { d: DeviceView; sync: DeviceSyncStatus | null; busy: boolean }) {
  const titleId = useId();
  const formatId = useId();
  const budgetId = useId();
  const folderId = useId();
  const [dialog, setDialog] = useState<"rename" | "forget" | null>(null);
  const defaultFormat = useSettings((s) => s.settings?.deviceFormat ?? "original");
  const plugged = d.root !== null;
  const mine = running(sync);

  const formatOptions: SelectOption<DeviceFormat | "default">[] = [
    { value: "default", label: `Default (${FORMAT_LABELS[defaultFormat]})` },
    ...FORMAT_ORDER.map((f) => ({ value: f, label: FORMAT_LABELS[f] })),
  ];
  const budgets = [...new Set([...BUDGETS, ...(d.budgetGb ? [d.budgetGb] : [])])].sort((a, b) => a - b);
  const budgetOptions: SelectOption<string>[] = [{ value: "0", label: "No limit" }, ...budgets.map((g) => ({ value: String(g), label: `${g} GB` }))];

  const chooseFolder = async () => {
    if (!d.root) return;
    try {
      const f = await backend.devicesChooseFolder(d.root);
      if (f !== null) await devices.update(d.id, { folder: f });
    } catch (e) {
      toast.error((e as AppError)?.message ?? "Couldn't use that folder.");
    }
  };

  const meta = plugged
    ? [`Plugged in (${d.root})`, spaceLine(d.free, d.total)].filter(Boolean).join(" · ")
    : `Not plugged in${d.lastSync ? ` · Last synced ${timeAgo(d.lastSync).toLowerCase()}` : ""}`;
  const usedPct = d.total && d.free !== null ? Math.round(((d.total - d.free) / d.total) * 100) : null;

  return (
    <section className="device-card" aria-labelledby={titleId} data-device={d.id}>
      <header className="device-card__head">
        <span className={`device-card__icon ${plugged ? "is-plugged" : ""}`} aria-hidden="true">
          <HardDrive size={24} />
        </span>
        <div className="device-card__heading">
          <h2 id={titleId} className="device-card__title">
            {d.name}
          </h2>
          <p className="device-card__meta">{meta}</p>
        </div>
        <div className="device-card__actions">
          <Button
            variant="primary"
            size="sm"
            icon={<RefreshCw size={16} />}
            disabled={!plugged || busy || d.selection.length === 0}
            onClick={() => void devices.sync(d.id)}
          >
            Sync now
          </Button>
          <IconButton
            size="sm"
            label={`More options for ${d.name}`}
            icon={<MoreHorizontal size={20} />}
            onClick={(e) =>
              openContextMenu(
                `Options for ${d.name}`,
                [
                  { label: "Rename", onSelect: () => setDialog("rename") },
                  { label: "Forget this device", disabled: mine, onSelect: () => setDialog("forget") },
                ],
                e.currentTarget,
              )
            }
          />
        </div>
      </header>

      {usedPct !== null && (
        <div className="device-space" role="img" aria-label={`${usedPct}% of the device is used`}>
          <span className="device-space__used" style={{ width: `${usedPct}%` }} />
        </div>
      )}

      {sync && <SyncPanel s={sync} />}

      <div className="device-card__grid">
        <div className="device-field">
          <span id={formatId} className="device-field__label">
            Format
          </span>
          <Select value={d.format ?? "default"} options={formatOptions} labelledBy={formatId} onChange={(v) => void devices.update(d.id, { format: v })} />
        </div>
        <div className="device-field">
          <span id={budgetId} className="device-field__label">
            Size limit
          </span>
          <Select value={String(d.budgetGb ?? 0)} options={budgetOptions} labelledBy={budgetId} onChange={(v) => void devices.update(d.id, { budgetGb: Number(v) })} />
        </div>
        <div className="device-field">
          <span id={folderId} className="device-field__label">
            Music folder
          </span>
          <span className="device-field__value">
            <span className="truncate">
              {d.folder || "The top of the drive"}
            </span>
            {plugged && (
              <Button size="sm" variant="ghost" onClick={() => void chooseFolder()}>
                Change
              </Button>
            )}
          </span>
        </div>
      </div>

      <Selection d={d} />

      {dialog === "rename" && <RenameDeviceDialog device={d} onClose={() => setDialog(null)} />}
      {dialog === "forget" && <ForgetDeviceDialog device={d} onClose={() => setDialog(null)} />}
    </section>
  );
}

function Selection({ d }: { d: DeviceView }) {
  const headingId = useId();
  const playlists = useApp((s) => s.playlists) ?? [];
  const hasLiked = d.selection.some((s) => s.kind === "liked");
  const chosen = new Set(d.selection.filter((s) => s.kind === "playlist").map((s) => s.id));
  const addable = playlists.filter((p) => !chosen.has(p.id));
  const remove = (s: DeviceItem) => void devices.select(d.id, s, false);
  return (
    <div className="device-sel">
      <h3 id={headingId} className="device-sel__title">
        On this device
      </h3>
      {d.selection.length === 0 ? (
        <p className="device-sel__empty">Nothing yet. Add Liked Songs or a playlist here, or use Send to device in any playlist's, album's or artist's menu.</p>
      ) : (
        <ol className="device-sel__list" aria-labelledby={headingId}>
          {d.selection.map((s) => {
            const Icon = KIND_ICON[s.kind];
            return (
              <li key={`${s.kind}:${s.id}`} className="device-sel__item">
                <Icon size={16} aria-hidden="true" className="device-sel__icon" />
                <span className="device-sel__name truncate">{s.title}</span>
                <span className="device-sel__kind">{KIND_LABEL[s.kind]}</span>
                <IconButton size="sm" label={`Take ${s.title} off ${d.name}`} icon={<X size={16} />} onClick={() => remove(s)} />
              </li>
            );
          })}
        </ol>
      )}
      <div className="device-sel__add">
        {!hasLiked && (
          <Button size="sm" icon={<Heart size={16} />} onClick={() => void devices.select(d.id, { kind: "liked", id: "", title: "Liked Songs" }, true)}>
            Add Liked Songs
          </Button>
        )}
        {addable.length > 0 && (
          <Select
            value=""
            label={`Add a playlist to ${d.name}`}
            options={[{ value: "", label: "Add a playlist…" }, ...addable.map((p) => ({ value: p.id, label: p.title }))]}
            onChange={(id) => {
              const p = addable.find((x) => x.id === id);
              if (p) void devices.select(d.id, { kind: "playlist", id: p.id, title: p.title }, true);
            }}
          />
        )}
      </div>
    </div>
  );
}

function SyncPanel({ s }: { s: DeviceSyncStatus }) {
  const live = running(s);
  const pct = s.total > 0 ? Math.round((s.done / s.total) * 100) : 0;
  return (
    <div className={`device-sync is-${s.phase}`}>
      {live && <progress className="device-sync__bar" max={Math.max(1, s.total)} value={s.done} aria-label={`Sync progress, ${pct}%`} />}
      <p className="device-sync__line" role={s.phase === "failed" ? "alert" : "status"}>
        {progressLine(s)}
      </p>
      {live && s.current && <p className="device-sync__current truncate">{s.current}</p>}
      {live && (
        <div className="device-sync__actions">
          {s.phase === "paused" ? (
            <Button size="sm" icon={<Play size={16} />} onClick={() => void devices.pause(false)}>
              Resume
            </Button>
          ) : (
            <Button size="sm" icon={<Pause size={16} />} onClick={() => void devices.pause(true)}>
              Pause
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={() => void devices.cancel()}>
            Cancel
          </Button>
        </div>
      )}
      {s.notice && <p className="device-sync__note">{s.notice}</p>}
      {s.leftOutSongs > 0 && (
        <p className="device-sync__note">
          Didn't fit: {s.leftOut.map((l) => `${l.title} (${l.songs} ${l.songs === 1 ? "song" : "songs"})`).join(", ")}. Raise the size limit or choose less.
        </p>
      )}
      {s.failed.length > 0 && (
        <details className="device-sync__failed">
          <summary>
            {s.failed.length} {s.failed.length === 1 ? "song" : "songs"} couldn't be copied
          </summary>
          <ul>
            {s.failed.map((f, i) => (
              <li key={i}>
                {f.title}: {f.error}
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

