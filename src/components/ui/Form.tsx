import {
  forwardRef,
  useId,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
} from "react";
import { cx } from "../../lib/cx";
import styles from "./Form.module.css";

export interface FieldProps {
  label: string;
  hint?: ReactNode;
  optional?: boolean;
  grow?: boolean;
  className?: string;
  /** Receives the id to put on the control so the label is associated with it. */
  children: (id: string) => ReactNode;
}

/** Label + control + hint, stacked. */
export function Field({ label, hint, optional, grow, className, children }: FieldProps) {
  const id = useId();
  return (
    <div className={cx(styles.field, grow && styles.grow, className)}>
      <label className={styles.label} htmlFor={id}>
        {label}
        {optional && <span className={styles.optional}> (optional)</span>}
      </label>
      {children(id)}
      {hint && <span className={styles.hint}>{hint}</span>}
    </div>
  );
}

export function FieldRow({ children }: { children: ReactNode }) {
  return <div className={styles.row}>{children}</div>;
}

export interface TextInputProps extends InputHTMLAttributes<HTMLInputElement> {
  mono?: boolean;
}

export const TextInput = forwardRef<HTMLInputElement, TextInputProps>(function TextInput(
  { mono, className, ...rest },
  ref,
) {
  return (
    <input
      ref={ref}
      className={cx(styles.control, mono && styles.mono, className)}
      spellCheck={false}
      {...rest}
    />
  );
});

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  /** Shown but can't be picked. */
  disabled?: boolean;
}

export interface SelectProps<T extends string> extends Omit<
  SelectHTMLAttributes<HTMLSelectElement>,
  "onChange" | "value"
> {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  className,
  ...rest
}: SelectProps<T>) {
  return (
    <select
      className={cx(styles.control, className)}
      value={value}
      onChange={(e) => onChange(e.target.value as T)}
      {...rest}
    >
      {options.map((o) => (
        <option key={o.value} value={o.value} disabled={o.disabled}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function Checkbox({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className={styles.checkbox}>
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      {label}
    </label>
  );
}

export interface SegmentedControlProps<T extends string> {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  ariaLabel: string;
}

export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  ariaLabel,
}: SegmentedControlProps<T>) {
  return (
    <div className={styles.segmented} role="radiogroup" aria-label={ariaLabel}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          className={cx(styles.segment, o.value === value && styles.segmentActive)}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
