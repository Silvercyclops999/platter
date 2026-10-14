import { ChevronDown } from "lucide-react";

export interface SelectOption<T extends string> {
  value: T;
  label: string;
}

export interface SelectProps<T extends string> {
  value: T;
  options: readonly SelectOption<T>[];
  onChange: (v: T) => void;
  /** Accessible name when there's no visible label to point at. */
  label?: string;
  labelledBy?: string;
  describedBy?: string;
  disabled?: boolean;
  id?: string;
}

/** Dropdown: a native select (keyboard and screen readers for free) styled
 *  as a pill. */
export function Select<T extends string>({ value, options, onChange, label, labelledBy, describedBy, disabled, id }: SelectProps<T>) {
  return (
    <span className={`select ${disabled ? "is-disabled" : ""}`}>
      <select
        id={id}
        className="select__input"
        value={value}
        disabled={disabled}
        aria-label={labelledBy ? undefined : label}
        aria-labelledby={labelledBy}
        aria-describedby={describedBy}
        onChange={(e) => onChange(e.target.value as T)}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <ChevronDown size={16} aria-hidden="true" className="select__icon" />
    </span>
  );
}
