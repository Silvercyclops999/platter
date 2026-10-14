import { CircleAlert, CloudOff, Info, RefreshCw, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";

export type BannerKind = "info" | "warning" | "error" | "offline" | "progress";

export interface BannerProps {
  kind: BannerKind;
  children: ReactNode;
  action?: ReactNode;
}

const ICONS = {
  info: Info,
  warning: TriangleAlert,
  error: CircleAlert,
  offline: CloudOff,
  progress: RefreshCw,
};

/** Full-width message strip at the top of the main view (connection states). */
export function Banner({ kind, children, action }: BannerProps) {
  const Icon = ICONS[kind];
  return (
    <div className={`banner banner--${kind}`} role={kind === "error" || kind === "offline" ? "alert" : "status"}>
      <Icon size={16} aria-hidden="true" className="banner__icon" />
      <span className="banner__msg">{children}</span>
      {action && <span className="banner__action">{action}</span>}
    </div>
  );
}
