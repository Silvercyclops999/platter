/**
 * The sleep timer in the player bar: a moon that opens the choices as a
 * menu, lit while a timer runs, its tooltip saying what's left. The core
 * keeps the time (stores/sleep). While this window controls another device
 * it hides: the timer belongs to the device that plays.
 */
import { Moon } from "lucide-react";
import { useEffect, useState } from "react";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { usePlayer } from "../stores/player";
import { SLEEP_CHOICES, cancelSleep, sameChoice, sleepTip, startSleep, useSleep, type SleepChoice } from "../stores/sleep";
import type { MenuItem } from "../stores/ui";

/** The desktop's lengths: the phone's, with two long ones for a custom length. */
const DESKTOP_CHOICES: { label: string; choice: SleepChoice }[] = [
  ...SLEEP_CHOICES.filter((c) => c.choice.kind === "minutes"),
  { label: "90 minutes", choice: { kind: "minutes", minutes: 90 } },
  { label: "2 hours", choice: { kind: "minutes", minutes: 120 } },
  ...SLEEP_CHOICES.filter((c) => c.choice.kind !== "minutes"),
];

/** The menu's items, for the button and the menu bar. */
export function sleepMenuItems(): MenuItem[] {
  const timer = useSleep.getState().timer;
  const hasTrack = !!usePlayer.getState().state?.track;
  const items: MenuItem[] = DESKTOP_CHOICES.map((c, i) => ({
    heading: i === 0 ? "Stop playing after" : undefined,
    label: c.label,
    icon: null,
    checked: sameChoice(timer, c.choice),
    disabled: c.choice.kind !== "minutes" && !hasTrack,
    onSelect: () => void startSleep(c.choice),
  }));
  if (timer) items.push({ label: "Turn off the timer", icon: null, separator: true, onSelect: () => void cancelSleep() });
  return items;
}

export function SleepButton() {
  const timer = useSleep((s) => s.timer);
  const remote = usePlayer((s) => !!s.remote);
  // The tooltip counts down while a timer in minutes runs.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (timer?.kind !== "minutes") return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 15_000);
    return () => clearInterval(t);
  }, [timer]);
  if (remote) return null;
  return (
    <IconButton
      label={timer ? `Sleep timer: ${sleepTip(timer, now)}` : "Sleep timer"}
      tooltip={sleepTip(timer, now)}
      pressed={!!timer}
      icon={<Moon size={17} />}
      className="sleep-button"
      onClick={(e) => openContextMenu("Sleep timer", sleepMenuItems(), e.currentTarget)}
    />
  );
}
