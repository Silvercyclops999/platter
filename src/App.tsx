import { lazy, Suspense, useEffect, useRef } from "react";
import { installReducedMotion, setMotionPreference } from "./design/motion";
import { installPageVisibility } from "./design/pageVisibility";
import { installOverlayScrollbars } from "./design/scrollbars";
import { installTheme, motionPreference } from "./stores/theme";
import { SignInPage, ServerPickPage } from "./pages/SignInPage";
import { AppShell } from "./shell/AppShell";
import { connectApp, useApp } from "./stores/app";
import { connectFileTags } from "./stores/fileTags";
import { connectLikes } from "./stores/likes";
import { connectMiniplayer, isMiniplayerWindow } from "./stores/miniplayer";
import { connectPlayer } from "./stores/player";
import { connectShare } from "./stores/share";
import { loadSettings, useSettings } from "./stores/settings";
import { backend } from "./lib/backend";
import { useUi } from "./stores/ui";
import { offerFirstRun } from "./guides/offer";
import { connectSwitching } from "./stores/switching";
import { SwitchingScreen } from "./shell/SwitchingScreen";
import { READY_WAIT_FOR_AUTH_MS, signalReady } from "./lib/firstPaint";

// Only the miniplayer window needs it.
const Miniplayer = lazy(() => import("./shell/Miniplayer").then((m) => ({ default: m.Miniplayer })));

/**
 * The miniplayer window: only the player, likes and settings stores, fed by
 * the same core events as the main window.
 */
export function MiniplayerApp() {
  useEffect(() => installReducedMotion(), []);
  useEffect(() => installPageVisibility(), []);
  useEffect(() => installTheme(), []);
  const reduceMotion = useSettings(motionPreference);
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
  return (
    <Suspense fallback={null}>
      <Miniplayer />
    </Suspense>
  );
}

/** Root: the miniplayer window gets its own page; the main window wires everything. */
export default function App() {
  return isMiniplayerWindow() ? <MiniplayerApp /> : <MainApp />;
}

/** The main window: wires stores to the core, then gates on auth. */
function MainApp() {
  useEffect(() => installReducedMotion(), []);
  useEffect(() => installPageVisibility(), []);
  useEffect(() => installTheme(), []);
  const reduceMotion = useSettings(motionPreference);
  useEffect(() => setMotionPreference(reduceMotion), [reduceMotion]);
  useEffect(() => installOverlayScrollbars(), []);
  useEffect(() => connectApp(), []);
  useEffect(() => connectPlayer(), []);
  useEffect(() => connectLikes(), []);
  useEffect(() => connectShare(), []);
  useEffect(() => connectFileTags(), []);
  useEffect(() => connectMiniplayer(), []);
  useEffect(() => connectSwitching(), []);
  const auth = useApp((s) => s.auth);
  // The window is shown once the page has drawn in the cached look, with
  // the right page (library or sign-in) when the sign-in is known in time.
  const authKnown = auth !== null;
  useEffect(() => {
    if (authKnown) return void signalReady();
    const t = setTimeout(() => void signalReady(), READY_WAIT_FOR_AUTH_MS);
    return () => clearTimeout(t);
  }, [authKnown]);
  const servers = useApp((s) => s.servers);
  const signInOpen = useUi((s) => s.signInOpen);
  // A Jellyfin server is enough to use the app without a Plex account.
  const otherServer = !!servers?.some((s) => s.kind !== "plex");
  // First run: right after signing in for the first time, offer the Get started guide (or "Pick a look").
  const sawSignIn = useRef(false);
  if (auth && auth.state !== "signedIn" && !auth.reauthRequired) sawSignIn.current = true;
  useEffect(() => {
    if (auth?.state === "signedIn") useUi.getState().setSignInOpen(false);
    if (auth?.state === "signedIn" && sawSignIn.current) {
      sawSignIn.current = false;
      offerFirstRun();
    }
  }, [auth?.state]);

  // Until auth status arrives, show the shell frame (no spinner, no flash of sign-in).
  // When Plex rejected the saved sign-in, the app stays open (cached music
  // still plays) and the sign-in screen opens from the banner.
  // Without Plex, the saved servers decide (until they're known, the frame shows).
  let page;
  if (auth && auth.state !== "signedIn" && servers !== null && ((!auth.reauthRequired && !otherServer) || signInOpen))
    page = <SignInPage onBack={auth.reauthRequired || otherServer ? () => useUi.getState().setSignInOpen(false) : undefined} />;
  else if (auth?.needsServerPick) page = <ServerPickPage />;
  else page = <AppShell />;
  return (
    <>
      {page}
      <SwitchingScreen />
    </>
  );
}
