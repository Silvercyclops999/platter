/**
 * Where this phone's sound goes: its own speaker, or headphones, a
 * Bluetooth or AirPlay speaker, a car. Read once, then kept by the core's
 * event on every change. Off iPhone the core answers null.
 */
import { Airplay, Car, Headphones, Speaker, Tv, Usb, type LucideIcon } from "lucide-react";
import { useEffect } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { AudioRoute } from "../lib/types";

export const useAudioRoute = create<{ route: AudioRoute | null }>()(() => ({ route: null }));

let watching = false;
export function useWatchRoute() {
  useEffect(() => {
    if (watching) return;
    watching = true;
    void backend.listen("audio://route", (route) => useAudioRoute.setState({ route })).catch(() => {});
    void backend
      .audioRoute()
      .then((route) => useAudioRoute.setState({ route }))
      .catch(() => {});
  }, []);
}

/** Test hook: forget the watch so a fresh render reads the route again. */
export function resetRouteWatch() {
  watching = false;
  useAudioRoute.setState({ route: null });
}

const KIND: Record<AudioRoute["kind"], { label: string; Icon: LucideIcon } | null> = {
  speaker: null,
  receiver: null,
  headphones: { label: "Headphones", Icon: Headphones },
  bluetooth: { label: "Bluetooth", Icon: Speaker },
  airplay: { label: "AirPlay", Icon: Airplay },
  car: { label: "Car", Icon: Car },
  usb: { label: "USB", Icon: Usb },
  hdmi: { label: "HDMI", Icon: Tv },
  other: { label: "Connected", Icon: Speaker },
};

/** The phone's own output when sound leaves it: "Bluetooth · JBL Flip 5" and its icon; null on the phone's speaker. */
export function routeLine(route: AudioRoute | null): { line: string; Icon: LucideIcon } | null {
  if (!route) return null;
  const k = KIND[route.kind];
  if (!k) return null;
  // Bluetooth headphones say so by name; show them with the headphones mark.
  const Icon = route.kind === "bluetooth" && /airpods|buds|headphone|beats/i.test(route.name) ? Headphones : k.Icon;
  return { line: route.name ? `${k.label} · ${route.name}` : k.label, Icon };
}
