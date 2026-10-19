import { lazy, Suspense, useEffect, useRef } from "react";
import { installReducedMotion, setMotionPreference } from "./design/motion";
import { installPageVisibility } from "./design/pageVisibility";
import { installOverlayScrollbars } from "./design/scrollbars";
import { installTheme, motionPreference } from "./stores/theme";
import { checkFormWithCore, usePhoneForm } from "./phone/form";
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

// Each form loads only its own shell and sign-in screens: the phone never
// fetches the computer's shell code, and the computer never fetches the phone's.
// The fallback is empty: the frame has no spinner, and the window is shown only once drawn.
const AppShell = lazy(() => import("./shell/AppShell").then((m) => ({ default: m.AppShell })));
const SignInPage = lazy(() => import("./pages/SignInPage").then((m) => ({ default: m.SignInPage })));
const ServerPickPage = lazy(() => import("./pages/SignInPage").then((m) => ({ default: m.ServerPickPage })));
const PhoneShell = lazy(() => import("./phone/PhoneShell").then((m) => ({ default: m.PhoneShell })));
const PhoneSignIn = lazy(() => import("./phone/PhoneSignIn").then((m) => ({ default: m.PhoneSignIn })));
const PhoneServerPick = lazy(() => import("./phone/PhoneSignIn").then((m) => ({ default: m.PhoneServerPick })));

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
  // A phone scrolls with its own thin bars; the computer's overlay bars stay off.
  useEffect(() => (usePhoneForm.getState().phone ? undefined : installOverlayScrollbars()), []);
  useEffect(() => connectApp(), []);
  useEffect(() => connectPlayer(), []);
  useEffect(() => connectLikes(), []);
  useEffect(() => connectShare(), []);
  useEffect(() => connectFileTags(), []);
  useEffect(() => connectMiniplayer(), []);
  useEffect(() => connectSwitching(), []);
  useEffect(() => void checkFormWithCore(), []);
  const isPhone = usePhoneForm((s) => s.phone);
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
      // The computer's Get started guide is about windows and layouts; the phone starts ready.
      if (!usePhoneForm.getState().phone) offerFirstRun();
    }
  }, [auth?.state]);

  // Until auth status arrives, show the shell frame (no spinner, no flash of sign-in).
  // When Plex rejected the saved sign-in, the app stays open (cached music
  // still plays) and the sign-in screen opens from the banner.
  // Without Plex, the saved servers decide (until they're known, the frame shows).
  let page;
  if (auth && auth.state !== "signedIn" && servers !== null && ((!auth.reauthRequired && !otherServer) || signInOpen)) {
    const onBack = auth.reauthRequired || otherServer ? () => useUi.getState().setSignInOpen(false) : undefined;
    page = isPhone ? <PhoneSignIn onBack={onBack} /> : <SignInPage onBack={onBack} />;
  } else if (auth?.needsServerPick) page = isPhone ? <PhoneServerPick /> : <ServerPickPage />;
  else page = isPhone ? <PhoneShell /> : <AppShell />;
  return (
    <>
      <Suspense fallback={null}>{page}</Suspense>
      <SwitchingScreen />
    </>
  );
}
