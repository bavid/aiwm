interface NumFieldProps {
  label: string;
  value: number;
  step: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
}

/** A labelled numeric input used by the Image and Video studios. The parent
 *  owns clamping/snapping — this only forwards finite numbers. */
export function NumField({ label, value, step, min, max, onChange }: NumFieldProps) {
  return (
    <label className="imgform__field">
      <span>{label}</span>
      <input
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
    </label>
  );
}
