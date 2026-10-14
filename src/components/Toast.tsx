import { CircleAlert, Info, X } from "lucide-react";
import { useToasts } from "../stores/toasts";
import { Art } from "./Art";

/**
 * Bottom-center toast stack (above the player bar). It is the app's one
 * polite status region, always present, so screen readers read each toast
 * ("Added to queue", errors too) as it appears without interrupting.
 */
export function Toaster() {
  const toasts = useToasts((s) => s.toasts);
  const dismiss = useToasts((s) => s.dismiss);
  return (
    <div className="toaster" role="status" aria-label="Notifications" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast--${t.kind} ${t.art ? "toast--art" : ""}`}>
          {t.art ? (
            <span className="toast__art" aria-hidden="true">
              <Art path={t.art.path} size={24} kind={t.art.kind} round={t.art.kind === "artist"} />
            </span>
          ) : (
            <span className="toast__icon" aria-hidden="true">
              {t.kind === "error" ? <CircleAlert size={16} /> : <Info size={16} />}
            </span>
          )}
          <span className="toast__msg">{t.message}</span>
          {t.action && (
            <button
              type="button"
              className="toast__action"
              onClick={() => {
                t.action?.run();
                dismiss(t.id);
              }}
            >
              {t.action.label}
            </button>
          )}
          <button type="button" className="toast__close" aria-label="Dismiss notification" onClick={() => dismiss(t.id)}>
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      ))}
    </div>
  );
}
