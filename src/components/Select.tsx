import { Check, ChevronDown, ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { createPortal } from "react-dom";
import { usePhoneForm } from "../phone/form";

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
 *  as a pill. On the phone it's a row showing the choice, which opens a page
 *  of the choices instead. */
export function Select<T extends string>(props: SelectProps<T>) {
  const phone = usePhoneForm((x) => x.phone);
  if (phone) return <PhoneChoice {...props} />;
  const { value, options, onChange, label, labelledBy, describedBy, disabled, id } = props;
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

/** The phone's choice: the value and a chevron; a tap pushes a page of the choices, a tap there picks and comes back. */
function PhoneChoice<T extends string>({ value, options, onChange, label, labelledBy, describedBy, disabled, id }: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const valueId = useId();
  const current = options.find((o) => o.value === value)?.label ?? "";
  // The page's title: the row's own title when there is one.
  const [title, setTitle] = useState(label ?? "");
  useEffect(() => {
    if (open && labelledBy) setTitle(document.getElementById(labelledBy)?.textContent?.trim() || label || "");
  }, [open, labelledBy, label]);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);
  const host = typeof document !== "undefined" ? (document.querySelector(".phone") ?? document.body) : null;
  return (
    <>
      <button
        type="button"
        id={id}
        className="ph-choice"
        disabled={disabled}
        aria-haspopup="dialog"
        aria-label={labelledBy ? undefined : label}
        aria-labelledby={labelledBy ? `${labelledBy} ${valueId}` : undefined}
        aria-describedby={describedBy}
        onClick={() => setOpen(true)}
      >
        <span id={valueId} className="ph-choice__value">
          {current}
        </span>
        <ChevronRight size={18} aria-hidden="true" className="ph-choice__chev" />
      </button>
      {open &&
        host &&
        createPortal(
          <div className="ph-choice-page" role="dialog" aria-modal="true" aria-label={title || "Choose"}>
            <div className="ph-choice-page__bar">
              <button type="button" className="ph-icon-btn" aria-label="Back" onClick={() => setOpen(false)}>
                <ChevronLeft size={26} aria-hidden="true" />
              </button>
              <span className="ph-choice-page__title">{title}</span>
              <span className="ph-icon-btn" aria-hidden="true" />
            </div>
            <ul className="ph-choice-page__list" role="radiogroup" aria-label={title || "Choices"}>
              {options.map((o) => (
                <li key={o.value}>
                  <button
                    type="button"
                    role="radio"
                    aria-checked={o.value === value}
                    className={`ph-choice-page__row ${o.value === value ? "is-on" : ""}`}
                    onClick={() => {
                      setOpen(false);
                      if (o.value !== value) onChange(o.value);
                    }}
                  >
                    <span>{o.label}</span>
                    {o.value === value && <Check size={20} aria-hidden="true" />}
                  </button>
                </li>
              ))}
            </ul>
          </div>,
          host,
        )}
    </>
  );
}
