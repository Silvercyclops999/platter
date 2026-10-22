import type { ButtonHTMLAttributes, ReactNode, Ref } from "react";
import { Tooltip } from "./Tooltip";

type Base = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> & { ref?: Ref<HTMLButtonElement> };

export interface ButtonProps extends Base {
  variant?: "primary" | "secondary" | "ghost";
  size?: "sm" | "md" | "lg";
  icon?: ReactNode;
  children?: ReactNode;
}

/** Pill button. Primary = accent fill; secondary = outline; ghost = text. */
export function Button({ variant = "secondary", size = "md", icon, children, className = "", type = "button", ...rest }: ButtonProps) {
  return (
    <button type={type} className={`btn btn--${variant} btn--${size} ${className}`} {...rest}>
      {icon && <span className="btn__icon" aria-hidden="true">{icon}</span>}
      {children && <span className="btn__label">{children}</span>}
    </button>
  );
}

export interface IconButtonProps extends Base {
  /** Accessible name; also the tooltip unless `tooltip` is given or false. */
  label: string;
  tooltip?: ReactNode | false;
  tooltipPlacement?: "top" | "bottom" | "right";
  icon: ReactNode;
  size?: "sm" | "md" | "lg" | "xl";
  variant?: "plain" | "filled" | "accent";
  /** Toggle buttons: sets aria-pressed and the active style. */
  pressed?: boolean;
  /** Focusable but inert (e.g. Phase 3 features); explains itself via tooltip. */
  softDisabled?: boolean;
}

export function IconButton({
  label,
  tooltip,
  tooltipPlacement = "top",
  icon,
  size = "md",
  variant = "plain",
  pressed,
  softDisabled,
  className = "",
  onClick,
  type = "button",
  ...rest
}: IconButtonProps) {
  const btn = (
    <button
      type={type}
      aria-label={label}
      aria-pressed={pressed}
      aria-disabled={softDisabled || undefined}
      className={`icon-btn icon-btn--${size} icon-btn--${variant} ${pressed ? "is-active" : ""} ${className}`}
      onClick={softDisabled ? (e) => e.preventDefault() : onClick}
      {...rest}
    >
      <span aria-hidden="true" className="icon-btn__glyph">{icon}</span>
    </button>
  );
  if (tooltip === false) return btn;
  return (
    <Tooltip label={tooltip ?? label} placement={tooltipPlacement} describe={tooltip !== undefined}>
      {btn}
    </Tooltip>
  );
}
