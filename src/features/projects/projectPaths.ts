const MAX_PROJECT_PATH_CHARS = 1024;

/**
 * Mirrors the Rust remote path normalization for display and pre-flight
 * validation. The backend validates the path again before any remote command.
 */
export function normalizeProjectPath(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed.startsWith("/") || trimmed.length > MAX_PROJECT_PATH_CHARS) {
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
  return normalized.length > MAX_PROJECT_PATH_CHARS ? null : normalized;
}
