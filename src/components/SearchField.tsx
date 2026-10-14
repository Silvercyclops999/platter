import { Search, X } from "lucide-react";
import type { KeyboardEvent, Ref } from "react";

export interface SearchFieldProps {
  value: string;
  onChange: (v: string) => void;
  onSubmit?: (v: string) => void;
  placeholder?: string;
  label?: string;
  ref?: Ref<HTMLInputElement>;
  className?: string;
  autoFocus?: boolean;
  onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void;
  /** Extra ARIA for combobox use (command palette). */
  inputProps?: Record<string, unknown>;
}

export function SearchField({
  value,
  onChange,
  onSubmit,
  placeholder = "What do you want to play?",
  label = "Search",
  ref,
  className = "",
  autoFocus,
  onKeyDown,
  inputProps,
}: SearchFieldProps) {
  return (
    <div className={`search-field ${className}`}>
      <Search size={18} aria-hidden="true" className="search-field__icon" />
      <input
        ref={ref}
        type="search"
        aria-label={label}
        placeholder={placeholder}
        value={value}
        autoFocus={autoFocus}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          onKeyDown?.(e);
          if (e.defaultPrevented) return;
          if (e.key === "Enter") onSubmit?.(value);
          if (e.key === "Escape" && value) {
            e.preventDefault();
            e.stopPropagation();
            onChange("");
          }
        }}
        {...inputProps}
      />
      {value && (
        <button type="button" className="search-field__clear" aria-label="Clear search" onClick={() => onChange("")}>
          <X size={16} aria-hidden="true" />
        </button>
      )}
    </div>
  );
}
