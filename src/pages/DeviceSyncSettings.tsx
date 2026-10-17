/**
 * The Devices section of Settings: the devices remembered on this computer
 * (rename, forget, or add one from the Devices page), the default format
 * and whether syncing removes songs taken off a device.
 */
import { HardDrive } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { ForgetDeviceDialog, RenameDeviceDialog } from "../components/DeviceDialogs";
import { Select, type SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import type { DeviceFormat, DeviceView } from "../lib/types";
import { FORMAT_LABELS, FORMAT_ORDER, startDevices, useDevices } from "../stores/devices";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import "./devices.css";

export const DEVICE_FORMAT_OPTIONS: SelectOption<DeviceFormat>[] = FORMAT_ORDER.map((f) => ({ value: f, label: FORMAT_LABELS[f] }));

export function DeviceFormatSetting({ ids }: { ids: { title: string; desc: string } }) {
  const v = useSettings((s) => s.settings?.deviceFormat);
  if (v === undefined) return <Skeleton width={120} height={32} radius="pill" />;
  return <Select value={v} options={DEVICE_FORMAT_OPTIONS} onChange={(f) => void updateSettings({ deviceFormat: f })} labelledBy={ids.title} describedBy={ids.desc} />;
}

export function OpenDevicesButton() {
  return (
    <Button size="sm" onClick={() => nav.go({ name: "devices" })}>
      Open Devices
    </Button>
  );
}

/** The devices, each with Rename and Forget. */
export function DeviceListSetting() {
  const view = useDevices((s) => s.view);
  const [dialog, setDialog] = useState<{ kind: "rename" | "forget"; d: DeviceView } | null>(null);
  useEffect(() => void startDevices(), []);
  if (!view) return <Skeleton width="60%" height={20} />;
  if (view.devices.length === 0) return <p className="device-settings__empty">No devices yet. Plug one in and add it on the Devices page.</p>;
  return (
    <>
      <ul className="device-settings" aria-label="Devices">
        {view.devices.map((d) => (
          <li key={d.id} className="device-settings__row">
            <HardDrive size={16} aria-hidden="true" className="device-settings__icon" />
            <span className="device-settings__name truncate">{d.name}</span>
            <span className="device-settings__state">{d.root ? "Plugged in" : "Not plugged in"}</span>
            <Button size="sm" variant="ghost" aria-label={`Rename ${d.name}`} onClick={() => setDialog({ kind: "rename", d })}>
              Rename
            </Button>
            <Button size="sm" variant="ghost" aria-label={`Forget ${d.name}`} onClick={() => setDialog({ kind: "forget", d })}>
              Forget
            </Button>
          </li>
        ))}
      </ul>
      {dialog?.kind === "rename" && <RenameDeviceDialog device={dialog.d} onClose={() => setDialog(null)} />}
      {dialog?.kind === "forget" && <ForgetDeviceDialog device={dialog.d} onClose={() => setDialog(null)} />}
    </>
  );
}
