import { useEffect, useId, useRef, useState } from "react";
import type { CleanupEntry, CleanupGroup } from "../../lib/ipc";
import { formatBytes } from "../../lib/units";
import { countLabel, ENTRY_LIST_CAP, GROUP_WHAT } from "./cleanup-groups";

type Props = {
  group: CleanupGroup;
  /** The ids of this group selected right now. */
  selected: readonly string[];
  /** Nothing can change while a preview or apply is in flight. */
  isDisabled: boolean;
  onToggleEntry: (id: string) => void;
  onToggleGroup: (on: boolean) => void;
};

/** One scan group: its label, what it is, the totals, a checkbox for the
 *  whole group (indeterminate while only some entries are picked) and one
 *  per entry. Long lists are capped behind "Show all". */
export function CleanupGroupCard({
  group,
  selected,
  isDisabled,
  onToggleEntry,
  onToggleGroup,
}: Props) {
  const [showAll, setShowAll] = useState(false);
  const allRef = useRef<HTMLInputElement>(null);
  const ids = useId();
  const listId = `${ids}-entries`;
  const titleId = `${ids}-title`;
  const total = group.entries.length;
  const picked = selected.length;
  const isAll = total > 0 && picked === total;
  const isSome = picked > 0 && !isAll;

  // The native mixed state has no attribute; it is a property.
  useEffect(() => {
    if (allRef.current) allRef.current.indeterminate = isSome;
  }, [isSome]);

  const visible = showAll ? group.entries : group.entries.slice(0, ENTRY_LIST_CAP);
  const hidden = total - visible.length;
  const totalRows = group.entries.reduce((n, e) => n + e.rows, 0);

  return (
    <section className="card set-group cleanup-group" aria-labelledby={titleId}>
      <header className="cleanup-group__head">
        <input
          ref={allRef}
          type="checkbox"
          className="cleanup-group__all"
          checked={isAll}
          disabled={isDisabled}
          aria-controls={listId}
          aria-label={`Select all in ${group.label}`}
          onChange={(e) => onToggleGroup(e.target.checked)}
        />
        <h3 id={titleId} className="cleanup-group__title">
          {group.label}
        </h3>
        <span className="cleanup-group__totals numeric">
          {countLabel(group.total_files, "file")} · {formatBytes(group.total_bytes)}
          {totalRows > 0 && ` · ${countLabel(totalRows, "entry", "entries")}`}
        </span>
      </header>
      {GROUP_WHAT[group.key] && <p className="cleanup-group__what muted">{GROUP_WHAT[group.key]}</p>}
      <ul id={listId} className="cleanup-entries">
        {visible.map((entry) => (
          <EntryRow
            key={entry.id}
            entry={entry}
            isChecked={selected.includes(entry.id)}
            isDisabled={isDisabled}
            onToggle={() => onToggleEntry(entry.id)}
          />
        ))}
      </ul>
      {hidden > 0 && (
        <button
          type="button"
          className="chip cleanup-group__more"
          aria-controls={listId}
          onClick={() => setShowAll(true)}
        >
          Show all {total.toLocaleString()} ({hidden.toLocaleString()} more)
        </button>
      )}
    </section>
  );
}

function EntryRow({
  entry,
  isChecked,
  isDisabled,
  onToggle,
}: {
  entry: CleanupEntry;
  isChecked: boolean;
  isDisabled: boolean;
  onToggle: () => void;
}) {
  return (
    <li className="cleanup-entry" data-checked={isChecked}>
      <label className="cleanup-entry__row">
        <input type="checkbox" checked={isChecked} disabled={isDisabled} onChange={onToggle} />
        <span className="cleanup-entry__label">{entry.label}</span>
        <span className="cleanup-entry__num numeric">{countLabel(entry.files, "file")}</span>
        <span className="cleanup-entry__num numeric">{formatBytes(entry.bytes)}</span>
        <span className="cleanup-entry__num numeric">
          {entry.rows > 0 ? countLabel(entry.rows, "entry", "entries") : ""}
        </span>
      </label>
      {entry.detail.length > 0 && (
        <ul className="cleanup-entry__detail">
          {entry.detail.map((line, i) => (
            <li key={`${i}-${line}`}>{line}</li>
          ))}
        </ul>
      )}
    </li>
  );
}
