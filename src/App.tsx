import { useEffect } from "react";
import { installReducedMotion } from "./design/motion";
import { SignInPage, ServerPickPage } from "./pages/SignInPage";
import { AppShell } from "./shell/AppShell";
import { connectApp, useApp } from "./stores/app";
import { connectPlayer } from "./stores/player";
import { useUi } from "./stores/ui";

/** Root: wires stores to the core, then gates on auth (spec 02). */
export default function App() {
  useEffect(() => installReducedMotion(), []);
  useEffect(() => connectApp(), []);
  useEffect(() => connectPlayer(), []);
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
