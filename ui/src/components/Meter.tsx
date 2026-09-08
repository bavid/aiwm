import "./meter.css";

type Load = "ok" | "warn" | "crit";

function loadOf(pct: number): Load {
  if (pct >= 90) return "crit";
  if (pct >= 70) return "warn";
  return "ok";
}

export function Meter({
  label,
  value,
  max,
  unit,
  format,
}: {
  label: string;
  value: number;
  max: number;
  unit?: string;
  format?: (v: number) => string;
}) {
  const pct = max > 0 ? Math.min(100, (value / max) * 100) : 0;
  const fmt = format ?? ((v: number) => v.toLocaleString());

  return (
    <div className="meter">
      <div className="meter__head">
        <span className="meter__label">{label}</span>
        <span className="meter__value numeric">
          {fmt(value)}
          {unit ? ` / ${fmt(max)} ${unit}` : ""}
        </span>
      </div>
      <div className="meter__track">
        <div
          className="meter__fill"
          data-load={loadOf(pct)}
          style={{ transform: `scaleX(${pct / 100})` }}
        />
      </div>
    </div>
  );
}
