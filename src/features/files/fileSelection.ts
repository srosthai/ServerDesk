export type FileSelection = ReadonlySet<string>;

export function toggleFileSelection(current: FileSelection, path: string): Set<string> {
  const next = new Set(current);
  if (next.has(path)) {
    next.delete(path);
  } else {
    next.add(path);
  }
  return next;
}

/**
 * Adds every path between an anchor and a target, in listing order, to the
 * current selection. The anchor is the last explicit selection. Returns a new
 * set only when the range changes the selection.
 */
export function addFileSelectionRange(
  current: FileSelection,
  orderedPaths: string[],
  anchor: string | null,
  target: string,
): Set<string> {
  const anchorIndex = anchor ? orderedPaths.indexOf(anchor) : -1;
  const targetIndex = orderedPaths.indexOf(target);
  if (targetIndex < 0 || anchorIndex < 0) {
    return new Set(current);
  }

  const start = Math.min(anchorIndex, targetIndex);
  const end = Math.max(anchorIndex, targetIndex);
  const next = new Set(current);
  for (let index = start; index <= end; index += 1) {
    next.add(orderedPaths[index]);
  }
  return next;
}

/** Drops selected paths that no longer exist after a listing refresh. */
export function retainFileSelection(current: FileSelection, orderedPaths: string[]): Set<string> {
  const existing = new Set(orderedPaths);
  const next = new Set([...current].filter((path) => existing.has(path)));
  return next.size === current.size ? new Set(current) : next;
}
