export const MAX_REMOTE_PATH_CHARS = 1024;

export type RemoteBreadcrumb = {
  name: string;
  path: string;
};

/**
 * Mirrors the Rust remote path normalization for display and pre-flight
 * validation. The backend normalizes and validates every request again.
 */
export function normalizeRemotePath(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed.startsWith("/") || trimmed.length > MAX_REMOTE_PATH_CHARS) {
    return null;
  }
  for (const character of trimmed) {
    const code = character.codePointAt(0) ?? 0;
    if (code < 0x20 || code === 0x7f) {
      return null;
    }
  }

  const segments: string[] = [];
  for (const segment of trimmed.split("/")) {
    if (!segment || segment === ".") {
      continue;
    }
    if (segment === "..") {
      return null;
    }
    segments.push(segment);
  }

  const normalized = segments.length > 0 ? `/${segments.join("/")}` : "/";
  return normalized.length > MAX_REMOTE_PATH_CHARS ? null : normalized;
}

/** Returns true when path is the selected root or a descendant of it. */
export function isRemotePathWithin(root: string, path: string): boolean {
  if (root === "/") {
    return path.startsWith("/");
  }

  return path === root || (path.startsWith(root) && path[root.length] === "/");
}

/** Returns the parent directory, never above the selected root. */
export function remoteParentPath(root: string, path: string): string | null {
  if (!isRemotePathWithin(root, path) || path === root) {
    return null;
  }

  const index = path.lastIndexOf("/");
  const parent = index <= 0 ? "/" : path.slice(0, index);
  return isRemotePathWithin(root, parent) ? parent : root;
}

function pathName(path: string): string {
  if (path === "/") {
    return "/";
  }
  const segments = path.split("/").filter(Boolean);
  return segments[segments.length - 1] ?? path;
}

/** Builds breadcrumbs from the selected root down to the current path. */
export function remoteBreadcrumbs(root: string, path: string): RemoteBreadcrumb[] {
  if (!isRemotePathWithin(root, path)) {
    return [{ name: pathName(root), path: root }];
  }

  const breadcrumbs: RemoteBreadcrumb[] = [{ name: pathName(root), path: root }];
  if (path === root) {
    return breadcrumbs;
  }

  const relative = root === "/" ? path.slice(1) : path.slice(root.length + 1);
  let current = root;
  for (const segment of relative.split("/").filter(Boolean)) {
    current = current === "/" ? `/${segment}` : `${current}/${segment}`;
    breadcrumbs.push({ name: segment, path: current });
  }

  return breadcrumbs;
}

/** Formats a remote byte size for display without changing the raw value. */
export function formatFileSize(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) {
    return "Unavailable";
  }
  if (bytes < 1024) {
    return `${bytes.toString()} B`;
  }

  const units = ["KiB", "MiB", "GiB", "TiB", "PiB"];
  let value = bytes / 1024;
  let unit = units[0];
  for (let index = 1; index < units.length && value >= 1024; index += 1) {
    value /= 1024;
    unit = units[index];
  }

  return `${value.toFixed(value >= 10 ? 0 : 1)} ${unit}`;
}
