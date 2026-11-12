export const UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
export const UPDATE_LAST_CHECK_KEY = "serverdesk-last-update-check";
export const UPDATE_RELEASES_URL = "https://github.com/srosthai/ServerDesk/releases/latest";

export type UpdateCheckTrigger = "startup" | "manual";

export type UpdateInfo = {
  version: string;
  currentVersion: string;
  notes: string;
  date: string | null;
};

export type UpdateProgress = {
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
};

export type UpdateErrorKind =
  | "offline"
  | "network"
  | "signature"
  | "permission"
  | "unsupported"
  | "install"
  | "relaunch"
  | "unknown";

export type UpdateError = {
  kind: UpdateErrorKind;
  message: string;
  details: string | null;
};

export type UpdateCheckEnvironment = {
  runtime: boolean;
  development: boolean;
  online: boolean;
  lastCheckedAt: number | null;
  now: number;
  trigger: UpdateCheckTrigger;
};

export type UpdateCheckDecision = "allowed" | "disabled" | "offline" | "recent";

export type UpdateErrorCopy = {
  title: string;
  description: string;
};

const semverPattern = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/;

type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[];
};

function parseVersion(version: string): ParsedVersion | null {
  const match = semverPattern.exec(version.trim());
  if (!match) {
    return null;
  }

  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease: match[4] ? match[4].split(".") : [],
  };
}

function comparePrerelease(left: string[], right: string[]): number {
  if (left.length === 0 && right.length === 0) {
    return 0;
  }
  if (left.length === 0) {
    return 1;
  }
  if (right.length === 0) {
    return -1;
  }

  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const leftPart = left[index];
    const rightPart = right[index];

    if (leftPart === undefined) {
      return -1;
    }
    if (rightPart === undefined) {
      return 1;
    }
    if (leftPart === rightPart) {
      continue;
    }

    const leftNumeric = /^\d+$/.test(leftPart);
    const rightNumeric = /^\d+$/.test(rightPart);
    if (leftNumeric && rightNumeric) {
      return Number(leftPart) - Number(rightPart);
    }
    if (leftNumeric) {
      return -1;
    }
    if (rightNumeric) {
      return 1;
    }
    return leftPart.localeCompare(rightPart);
  }

  return 0;
}

export function compareVersions(left: string, right: string): number {
  const parsedLeft = parseVersion(left);
  const parsedRight = parseVersion(right);

  if (!parsedLeft || !parsedRight) {
    return 0;
  }

  return (
    parsedLeft.major - parsedRight.major ||
    parsedLeft.minor - parsedRight.minor ||
    parsedLeft.patch - parsedRight.patch ||
    comparePrerelease(parsedLeft.prerelease, parsedRight.prerelease)
  );
}

export function isNewerVersion(candidate: string, current: string): boolean {
  const parsedCandidate = parseVersion(candidate);
  const parsedCurrent = parseVersion(current);

  if (!parsedCandidate || !parsedCurrent) {
    return candidate.trim() !== current.trim();
  }

  return compareVersions(candidate, current) > 0;
}

export function decideUpdateCheck({
  runtime,
  development,
  online,
  lastCheckedAt,
  now,
  trigger,
}: UpdateCheckEnvironment): UpdateCheckDecision {
  if (!runtime || development) {
    return "disabled";
  }

  if (!online) {
    return "offline";
  }

  if (
    trigger === "startup" &&
    lastCheckedAt !== null &&
    now - lastCheckedAt < UPDATE_CHECK_INTERVAL_MS
  ) {
    return "recent";
  }

  return "allowed";
}

export function createUpdateProgress(
  downloadedBytes: number,
  totalBytes: number | null,
): UpdateProgress {
  const safeDownloaded = Number.isFinite(downloadedBytes) ? Math.max(0, downloadedBytes) : 0;
  const safeTotal =
    typeof totalBytes === "number" && Number.isFinite(totalBytes) && totalBytes > 0
      ? totalBytes
      : null;
  const percent = safeTotal
    ? Math.min(100, Math.max(0, Math.round((safeDownloaded / safeTotal) * 100)))
    : null;

  return {
    downloadedBytes: safeDownloaded,
    totalBytes: safeTotal,
    percent,
  };
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }

  const units = ["B", "KB", "MB", "GB"];
  const exponent = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const value = bytes / 1024 ** exponent;
  const displayed = exponent === 0 ? Math.round(value) : Number(value.toFixed(value >= 10 ? 0 : 1));
  return `${displayed} ${units[exponent]}`;
}

export function formatVersionLabel(version: string): string {
  const trimmed = version.trim();
  if (!trimmed) {
    return "Unknown version";
  }

  return trimmed.startsWith("v") ? trimmed : `v${trimmed}`;
}

export function formatLastChecked(lastCheckedAt: number | null, now = Date.now()): string {
  if (!lastCheckedAt || !Number.isFinite(lastCheckedAt)) {
    return "Not checked yet";
  }

  const elapsed = Math.max(0, now - lastCheckedAt);
  if (elapsed < 60_000) {
    return "Checked just now";
  }
  if (elapsed < 60 * 60_000) {
    const minutes = Math.floor(elapsed / 60_000);
    return `Checked ${minutes} min ago`;
  }
  if (elapsed < 24 * 60 * 60_000) {
    const hours = Math.floor(elapsed / (60 * 60_000));
    return `Checked ${hours} hr ago`;
  }

  return `Checked on ${new Date(lastCheckedAt).toISOString().slice(0, 10)}`;
}

export function getUpdateErrorMessage(error: unknown): string {
  if (typeof error === "string" && error.trim().length > 0) {
    return error.trim();
  }

  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message.trim();
  }

  if (typeof error === "object" && error !== null) {
    const record = error as Record<string, unknown>;
    if (typeof record.message === "string" && record.message.trim().length > 0) {
      return record.message.trim();
    }
    if (typeof record.error === "string" && record.error.trim().length > 0) {
      return record.error.trim();
    }
  }

  return "The update operation failed for an unknown reason.";
}

export function classifyUpdateError(error: unknown): UpdateError {
  const message = getUpdateErrorMessage(error);
  const normalized = message.toLowerCase();

  let kind: UpdateErrorKind = "unknown";
  if (
    normalized.includes("signature") ||
    normalized.includes("minisign") ||
    normalized.includes("public key") ||
    normalized.includes("corrupt updater") ||
    normalized.includes("invalid updater format")
  ) {
    kind = "signature";
  } else if (
    normalized.includes("permission") ||
    normalized.includes("not allowed") ||
    normalized.includes("denied") ||
    normalized.includes("forbidden") ||
    normalized.includes("capability") ||
    normalized.includes("unauthorized") ||
    normalized.includes("401") ||
    normalized.includes("403")
  ) {
    kind = "permission";
  } else if (
    normalized.includes("unsupported") ||
    normalized.includes("not supported") ||
    normalized.includes("appimage") ||
    normalized.includes("bundle type") ||
    normalized.includes("platform")
  ) {
    kind = "unsupported";
  } else if (
    normalized.includes("offline") ||
    normalized.includes("no internet") ||
    normalized.includes("network is unreachable")
  ) {
    kind = "offline";
  } else if (
    normalized.includes("network") ||
    normalized.includes("failed to fetch") ||
    normalized.includes("error sending request") ||
    normalized.includes("could not connect") ||
    normalized.includes("connection refused") ||
    normalized.includes("timed out") ||
    normalized.includes("timeout") ||
    normalized.includes("dns") ||
    normalized.includes("resolve") ||
    normalized.includes("status code") ||
    normalized.includes("502") ||
    normalized.includes("503") ||
    normalized.includes("504") ||
    normalized.includes("404")
  ) {
    kind = "network";
  } else if (normalized.includes("install") || normalized.includes("replace")) {
    kind = "install";
  }

  return {
    kind,
    message,
    details: kind === "unknown" ? null : message,
  };
}

export function getUpdateErrorCopy(error: UpdateError): UpdateErrorCopy {
  switch (error.kind) {
    case "offline":
      return {
        title: "You are offline",
        description:
          "ServerDesk could not reach the update server. Connect to the internet and try again.",
      };
    case "network":
      return {
        title: "The update server could not be reached",
        description:
          "The check or download did not complete. Your current installation is unchanged; try again later.",
      };
    case "signature":
      return {
        title: "The update could not be verified",
        description:
          "ServerDesk rejected the update because its signature was missing or invalid. Nothing was installed.",
      };
    case "permission":
      return {
        title: "The update is not permitted",
        description:
          "ServerDesk does not have permission to check or install updates in this environment. Check the app permissions or install the release manually.",
      };
    case "unsupported":
      return {
        title: "In-app updates are not supported here",
        description:
          "This installation type cannot replace itself. Download the matching package from the release page instead.",
      };
    case "install":
      return {
        title: "The update could not be installed",
        description:
          "The download finished, but ServerDesk could not replace the installed build. Your current installation is unchanged; try again or install the release manually.",
      };
    case "relaunch":
      return {
        title: "The update installed but ServerDesk could not restart",
        description:
          "Quit ServerDesk and open it again to finish the update. If the version does not change, reinstall from the latest release.",
      };
    default:
      return {
        title: "The update check failed",
        description:
          "ServerDesk could not complete the update operation. Your current installation is unchanged.",
      };
  }
}

export function readLastUpdateCheck(): number | null {
  try {
    const raw = localStorage.getItem(UPDATE_LAST_CHECK_KEY);
    if (!raw) {
      return null;
    }

    const parsed = Number(raw);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  } catch {
    return null;
  }
}

export function writeLastUpdateCheck(timestamp: number) {
  try {
    localStorage.setItem(UPDATE_LAST_CHECK_KEY, String(timestamp));
  } catch {
    // Storage can be unavailable in restricted webviews; the check still works.
  }
}

export function clearLastUpdateCheck() {
  try {
    localStorage.removeItem(UPDATE_LAST_CHECK_KEY);
  } catch {
    // Ignore restricted storage.
  }
}
