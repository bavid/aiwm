import { useSyncExternalStore } from "react";

/** Which downloads were queued together by one package ("Download X +
 *  missing"), so the Downloads section can show them under the package's
 *  name. In memory only: the queue itself does not record a package, and a
 *  restart simply shows the files ungrouped again — each one still carries
 *  its own origin and family. */
export interface PackageGroup {
  id: string;
  label: string;
}

let groups: ReadonlyMap<string, PackageGroup> = new Map();
const listeners = new Set<() => void>();

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

const snapshot = () => groups;

let seq = 0;

/** Remember that these download ids belong to one package. A single file is
 *  not a group and is left alone. */
export function tagPackage(label: string, downloadIds: readonly string[]): void {
  const unique = [...new Set(downloadIds)];
  if (unique.length < 2) return;
  seq += 1;
  const group: PackageGroup = { id: `pkg-${seq}`, label };
  const next = new Map(groups);
  for (const id of unique) next.set(id, group);
  groups = next;
  for (const l of listeners) l();
}

/** download id → the package it was queued with. */
export function usePackageGroups(): ReadonlyMap<string, PackageGroup> {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
