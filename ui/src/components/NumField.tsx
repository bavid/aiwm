import { useId, type ReactNode } from "react";

interface NumFieldProps {
  label: string;
  value: number;
  step: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
  /** Renders the `?` help hint next to the label; gets the input's id so the
   *  hint can describe it. A render prop rather than `{area, setting}`, so
   *  the caller writes the hint element itself with literal props that
   *  `scripts/check-help.mjs` can verify. */
  hint?: (inputId: string) => ReactNode;
}

/** A labelled numeric input used by the Image and Video studios. The parent
 *  owns clamping/snapping — this only forwards finite numbers. The caption is
 *  a `<label for>` next to the optional hint, never wrapping it, so the
 *  hint's own name never joins the input's. */
export function NumField({ label, value, step, min, max, onChange, hint }: NumFieldProps) {
  const id = useId();
  return (
    <div className="imgform__field">
      <span>
        <label htmlFor={id}>{label}</label>
        {hint?.(id)}
      </span>
      <input
        id={id}
        type="number"
        value={value}
        step={step}
        min={min}
        max={max}
        onChange={(e) => {
          const n = Number(e.target.value);
          if (Number.isFinite(n)) onChange(n);
        }}
        spellCheck={false}
      />
    </div>
  );
}
