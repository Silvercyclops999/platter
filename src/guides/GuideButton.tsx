import { Button } from "../components/Button";
import { GUIDE_TITLES, guides, useGuides, type GuideId } from "../stores/guides";

/** Opens a setup guide from a feature's empty state ("Set up with a guide"). */
export function GuideButton({ id, label, variant = "secondary" }: { id: GuideId; label?: string; variant?: "primary" | "secondary" | "ghost" }) {
  const step = useGuides((x) => x.progress[id]?.step ?? 0);
  return (
    <Button size="sm" variant={variant} onClick={() => guides.open(id)} aria-label={label ?? `${step > 0 ? "Resume" : "Start"} the guide: ${GUIDE_TITLES[id]}`}>
      {label ?? (step > 0 ? "Resume the guide" : "Set up with a guide")}
    </Button>
  );
}
