export interface SwitchProps {
  checked: boolean;
  onChange: (v: boolean) => void;
  /** Accessible name (usually the visible label's text). */
  label?: string;
  /** id of the element that labels this switch. */
  labelledBy?: string;
  describedBy?: string;
  disabled?: boolean;
  id?: string;
}

/** On/off toggle (role="switch"). Space and Enter flip it, like a button. */
export function Switch({ checked, onChange, label, labelledBy, describedBy, disabled, id }: SwitchProps) {
  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={labelledBy ? undefined : label}
      aria-labelledby={labelledBy}
      aria-describedby={describedBy}
      disabled={disabled}
      className={`switch ${checked ? "is-on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="switch__thumb" aria-hidden="true" />
    </button>
  );
}
