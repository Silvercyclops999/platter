import { useEffect } from "react";
import { installReducedMotion, setMotionPreference } from "./design/motion";
import { installOverlayScrollbars } from "./design/scrollbars";
import { SignInPage, ServerPickPage } from "./pages/SignInPage";
import { AppShell } from "./shell/AppShell";
import { connectApp, useApp } from "./stores/app";
import { connectFileTags } from "./stores/fileTags";
import { connectLikes } from "./stores/likes";
import { connectMiniplayer, isMiniplayerWindow } from "./stores/miniplayer";
import { connectPlayer } from "./stores/player";
import { connectShare } from "./stores/share";
import { loadSettings, useSettings } from "./stores/settings";
import { Miniplayer } from "./shell/Miniplayer";
import { backend } from "./lib/backend";
import { useUi } from "./stores/ui";

/**
 * The miniplayer window: only the player, likes and settings stores, fed by
 * the same core events as the main window.
 */
export function MiniplayerApp() {
  useEffect(() => installReducedMotion(), []);
  const reduceMotion = useSettings((s) => s.settings?.reduceMotion ?? "system");
  useEffect(() => setMotionPreference(reduceMotion), [reduceMotion]);
  useEffect(() => connectPlayer(), []);
  useEffect(() => connectLikes(), []);
  useEffect(() => {
    void loadSettings();
    let un: (() => void) | null = null;
    let alive = true;
    backend.listen("settings://changed", (s) => useSettings.getState().setLocal(s)).then((u) => (alive ? (un = u) : u()));
    return () => {
      alive = false;
      un?.();
    };
  }, []);
  return <Miniplayer />;
}

/** Root: the miniplayer window gets its own page; the main window wires everything. */
export default function App() {
  return isMiniplayerWindow() ? <MiniplayerApp /> : <MainApp />;
}

/** The main window: wires stores to the core, then gates on auth. */
function MainApp() {
  useEffect(() => installReducedMotion(), []);
  const reduceMotion = useSettings((s) => s.settings?.reduceMotion ?? "system");
  useEffect(() => setMotionPreference(reduceMotion), [reduceMotion]);
  useEffect(() => installOverlayScrollbars(), []);
  useEffect(() => connectApp(), []);
  useEffect(() => connectPlayer(), []);
  useEffect(() => connectLikes(), []);
  useEffect(() => connectShare(), []);
  useEffect(() => connectFileTags(), []);
  useEffect(() => connectMiniplayer(), []);
  const auth = useApp((s) => s.auth);
  const signInOpen = useUi((s) => s.signInOpen);
  useEffect(() => {
    if (auth?.state === "signedIn") useUi.getState().setSignInOpen(false);
  }, [auth?.state]);

  // Until auth status arrives, show the shell frame (no spinner, no flash of sign-in).
  // When Plex rejected the saved sign-in, the app stays open (cached music
  // still plays) and the sign-in screen opens from the banner.
  if (auth && auth.state !== "signedIn" && (!auth.reauthRequired || signInOpen))
    return <SignInPage onBack={auth.reauthRequired ? () => useUi.getState().setSignInOpen(false) : undefined} />;
  if (auth?.needsServerPick) return <ServerPickPage />;
  return <AppShell />;
}
