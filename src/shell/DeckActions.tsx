/**
 * What a Stream Deck key asks the window for (`deck://action`): Listen
 * along starts a session or shows the open invite, and Request album opens
 * the usual Lidarr request dialog. The dialog's code loads the first time.
 */
import { lazy, Suspense, useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { DeckAction, LidarrRequestInput } from "../lib/types";
import { openInvite, startSession, useSocial } from "../stores/social";
import { toast } from "../stores/toasts";

const RequestDialog = lazy(() => import("../pages/LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

/** Carry out one Stream Deck action that needs the window. */
export function runDeckListen(): Promise<void> {
  const role = useSocial.getState().listen.role;
  if (role === "host") return openInvite();
  if (role === "guest") {
    toast.info("You're listening along with someone. Leave that session to host your own.");
    return Promise.resolve();
  }
  return startSession();
}

export function DeckActions() {
  const [input, setInput] = useState<LidarrRequestInput | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    let alive = true;
    const un = backend.listen("deck://action", (a: DeckAction) => {
      if (!alive) return;
      if (a.kind === "listenAlong") void runDeckListen();
      else {
        setInput(a.input);
        setOpen(true);
      }
    });
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  if (!input) return null;
  return (
    <Suspense fallback={null}>
      <RequestDialog
        open={open}
        input={input}
        onClose={() => setOpen(false)}
        onDone={() => {
          setOpen(false);
          toast.info(`Requested ${input.title}`);
        }}
      />
    </Suspense>
  );
}
