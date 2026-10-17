/**
 * Dialogs for Sync to a device, shared by the Devices page and Settings:
 * add a plugged-in drive as a device, rename one, and forget one.
 */
import { useEffect, useId, useRef, useState } from "react";
import { backend } from "../lib/backend";
import type { AppError, DeviceView, DriveVolume } from "../lib/types";
import { devices } from "../stores/devices";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import "./device-dialogs.css";

/** "WALKMAN (E:\)" or the drive's place when it has no name. */
export function driveName(v: Pick<DriveVolume, "label" | "root">): string {
  return v.label ? `${v.label} (${v.root})` : v.root;
}

export function AddDeviceDialog({ drive, onClose }: { drive: DriveVolume; onClose: () => void }) {
  const [name, setName] = useState(drive.label || "My player");
  const [folder, setFolder] = useState("Music");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameId = useId();
  const folderId = useId();
  const first = useRef<HTMLInputElement>(null);
  const choose = async () => {
    try {
      const f = await backend.devicesChooseFolder(drive.root);
      if (f !== null) setFolder(f);
    } catch (e) {
      setError((e as AppError)?.message ?? "Couldn't use that folder.");
    }
  };
  const add = async () => {
    if (!name.trim()) return setError("Give the device a name.");
    setBusy(true);
    const d = await devices.add(drive.root, name, folder);
    setBusy(false);
    if (d) onClose();
  };
  return (
    <Dialog open onClose={onClose} label="Add a device" className="confirm device-dialog" initialFocus={first}>
      <h2 className="confirm__title">Add {driveName(drive)}</h2>
      <p className="confirm__body">Platter copies music into the folder you pick and keeps track of what it put there. Files already on the device stay as they are.</p>
      <form
        className="device-dialog__form"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <label htmlFor={nameId} className="device-dialog__label">
          Name
        </label>
        <input id={nameId} ref={first} className="device-input" value={name} maxLength={60} spellCheck={false} onChange={(e) => setName(e.target.value)} />
        <label htmlFor={folderId} className="device-dialog__label">
          Music folder
        </label>
        <div className="device-dialog__folder">
          <input
            id={folderId}
            className="device-input"
            value={folder}
            placeholder="The top of the drive"
            spellCheck={false}
            onChange={(e) => setFolder(e.target.value)}
          />
          <Button size="sm" onClick={() => void choose()}>
            Choose…
          </Button>
        </div>
        {error && (
          <p className="confirm__error" role="alert">
            {error}
          </p>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" disabled={busy}>
            Add device
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function RenameDeviceDialog({ device, onClose }: { device: DeviceView; onClose: () => void }) {
  const [name, setName] = useState(device.name);
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.select(), []);
  const save = async () => {
    if (!name.trim() || name.trim() === device.name) return onClose();
    if (await devices.update(device.id, { name })) onClose();
  };
  return (
    <Dialog open onClose={onClose} label="Rename device" className="confirm device-dialog" initialFocus={input}>
      <h2 className="confirm__title">Rename {device.name}</h2>
      <form
        className="device-dialog__form"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <label htmlFor={id} className="device-dialog__label">
          Name
        </label>
        <input id={id} ref={input} className="device-input" value={name} maxLength={60} spellCheck={false} onChange={(e) => setName(e.target.value)} />
        <div className="confirm__actions">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit">
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function ForgetDeviceDialog({ device, onClose }: { device: DeviceView; onClose: () => void }) {
  return (
    <Dialog open onClose={onClose} label="Forget device" className="confirm">
      <h2 className="confirm__title">Forget {device.name}?</h2>
      <p className="confirm__body">Platter stops syncing to it on this computer. The music already on the device stays there.</p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            onClose();
            void devices.forget(device.id);
          }}
        >
          Forget
        </Button>
      </div>
    </Dialog>
  );
}
