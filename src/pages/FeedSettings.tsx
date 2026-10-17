import { Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { IconButton } from "../components/Button";
import { backend } from "../lib/backend";
import type { FeedStatus } from "../lib/types";
import { updateSettings } from "../stores/settings";
import { copyText } from "../stores/toasts";
import "./feed-settings.css";

/** The feed's status, kept current from `feed://status`. */
export function useFeedStatus(): FeedStatus | null {
  const [status, setStatus] = useState<FeedStatus | null>(null);
  useEffect(() => {
    let alive = true;
    backend.feedStatus().then(
      (s) => alive && setStatus(s),
      () => {},
    );
    const un = backend.listen("feed://status", (s) => alive && setStatus(s));
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  return status;
}

/** Under the feed's switch: where it is, or why it couldn't start. */
export function FeedNote({ status }: { status: FeedStatus | null }) {
  if (!status || status.state === "off") return null;
  if (status.state === "portInUse")
    return (
      <span className="info-key__error" role="alert">
        Port {status.port} is in use by another program. Pick another port below.
      </span>
    );
  if (status.state === "failed")
    return (
      <span className="info-key__error" role="alert">
        Couldn't open port {status.port}: {status.message}. Another app may be using it. Choose another port.
      </span>
    );
  const page = status.url.replace(/\.json$/, ".html");
  return (
    <span className="feed-note">
      <span>
        Data: <code>{status.url}</code>
      </span>
      <IconButton label="Copy the data address" size="sm" icon={<Copy size={14} />} onClick={() => copyText(status.url)} />
      <span>
        Overlay: <code>{page}</code>
      </span>
      <IconButton label="Copy the overlay address" size="sm" icon={<Copy size={14} />} onClick={() => copyText(page)} />
    </span>
  );
}

/** 1024 to 65535; saved on Enter or when leaving the field, so typing doesn't reopen the port each digit. */
export function FeedPortField({ value, ids }: { value: number; ids: { title: string; desc: string } }) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);
  const commit = () => {
    const n = Number(text);
    if (Number.isInteger(n) && n >= 1024 && n <= 65535) {
      if (n !== value) void updateSettings({ nowPlayingFeedPort: n });
    } else setText(String(value));
  };
  return (
    <input
      type="number"
      inputMode="numeric"
      min={1024}
      max={65535}
      step={1}
      className="number-field number-field--wide"
      value={text}
      aria-labelledby={ids.title}
      aria-describedby={ids.desc}
      onChange={(e) => setText(e.target.value)}
      onKeyDown={(e) => e.key === "Enter" && commit()}
      onBlur={commit}
    />
  );
}
