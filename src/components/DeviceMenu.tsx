/**
 * "Send to device" for a playlist's, album's, artist's or Liked Songs'
 * menu: a submenu of the devices on this computer. Picking one adds the
 * item to that device and syncs it now when it's plugged in. With no
 * devices yet it opens the Devices page.
 */
import { Check, HardDrive, Plus } from "lucide-react";
import { useEffect, useRef, type KeyboardEvent } from "react";
import { startDevices, sendToDevice, useDevices, type DeviceTarget } from "../stores/devices";
import { nav } from "../stores/router";
import type { MenuItem, SubmenuContext } from "../stores/ui";
import "./device-dialogs.css";

function DevicePicker({ target, ctx }: { target: DeviceTarget; ctx: SubmenuContext }) {
  const view = useDevices((s) => s.view);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => void startDevices(), []);
  const list = view?.devices ?? [];
  const has = (id: string) =>
    !!list.find((d) => d.id === id)?.selection.some((s) => s.kind === target.kind && (target.kind === "liked" || s.id === target.id));
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const els = Array.from(root.current?.querySelectorAll<HTMLElement>("[role='menuitem']") ?? []);
    const i = els.indexOf(document.activeElement as HTMLElement);
    els[(i + (e.key === "ArrowDown" ? 1 : -1) + els.length) % els.length]?.focus();
  };
  return (
    <div ref={root} className="device-picker" onKeyDown={onKeyDown}>
      {list.map((d) => (
        <button
          key={d.id}
          type="button"
          role="menuitem"
          tabIndex={-1}
          className="context-menu__item"
          onClick={() => {
            ctx.close();
            void sendToDevice(d.id, target);
          }}
        >
          {has(d.id) ? <Check size={16} aria-hidden="true" /> : <HardDrive size={16} aria-hidden="true" />}
          <span className="truncate">{d.name}</span>
          <span className="device-picker__state">{d.root ? "Plugged in" : "Not plugged in"}</span>
        </button>
      ))}
      {list.length > 0 && <div className="context-menu__sep" role="separator" />}
      <button
        type="button"
        role="menuitem"
        tabIndex={-1}
        className="context-menu__item"
        onClick={() => {
          ctx.close();
          nav.go({ name: "devices" });
        }}
      >
        <Plus size={16} aria-hidden="true" />
        <span className="truncate">{list.length ? "Manage devices" : "Add a device"}</span>
      </button>
    </div>
  );
}

export function sendToDeviceItem(target: DeviceTarget): MenuItem {
  return { label: "Send to device", icon: HardDrive, submenu: (ctx) => <DevicePicker target={target} ctx={ctx} /> };
}
