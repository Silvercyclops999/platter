import { useEffect } from "react";
import { installReducedMotion } from "./design/motion";
import { SignInPage, ServerPickPage } from "./pages/SignInPage";
import { AppShell } from "./shell/AppShell";
import { connectApp, useApp } from "./stores/app";
import { connectPlayer } from "./stores/player";

/** Root: wires stores to the core, then gates on auth (spec 02). */
export default function App() {
  useEffect(() => installReducedMotion(), []);
  useEffect(() => connectApp(), []);
  useEffect(() => connectPlayer(), []);
  const auth = useApp((s) => s.auth);

  // Until auth status arrives, show the shell frame (no spinner, no flash of sign-in).
  if (auth && auth.state !== "signedIn") return <SignInPage />;
  if (auth?.needsServerPick) return <ServerPickPage />;
  return <AppShell />;
}
