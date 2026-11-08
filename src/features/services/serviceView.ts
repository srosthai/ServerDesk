import type { ServiceActionResult, ServiceState, ServiceUnit } from "@/types/contracts";

export type ServiceSortKey = "name" | "state" | "description";
export type SortDirection = "asc" | "desc";
export type ServiceSort = {
  key: ServiceSortKey;
  direction: SortDirection;
};
export type ServiceStateFilter = "all" | ServiceState;

const stateSortOrder: Record<ServiceState, number> = {
  failed: 0,
  unknown: 1,
  active: 2,
  inactive: 3,
};

function compareNames(left: ServiceUnit, right: ServiceUnit) {
  return left.name.localeCompare(right.name, undefined, {
    numeric: true,
    sensitivity: "base",
  });
}

function compareStates(left: ServiceUnit, right: ServiceUnit) {
  return stateSortOrder[left.state] - stateSortOrder[right.state] || compareNames(left, right);
}

function compareDescriptions(left: ServiceUnit, right: ServiceUnit) {
  const leftDescription = left.description?.trim() ?? "";
  const rightDescription = right.description?.trim() ?? "";

  if (!leftDescription && rightDescription) {
    return 1;
  }
  if (leftDescription && !rightDescription) {
    return -1;
  }

  return (
    leftDescription.localeCompare(rightDescription, undefined, { sensitivity: "base" }) ||
    compareNames(left, right)
  );
}

function compareServices(left: ServiceUnit, right: ServiceUnit, sort: ServiceSort) {
  switch (sort.key) {
    case "state":
      return compareStates(left, right);
    case "description":
      return compareDescriptions(left, right);
    case "name":
      return compareNames(left, right);
  }
}

function matchesQuery(service: ServiceUnit, query: string) {
  if (!query) {
    return true;
  }

  return [
    service.name,
    service.description,
    service.activeState,
    service.subState,
    service.loadState,
  ]
    .filter((value): value is string => Boolean(value))
    .some((value) => value.toLowerCase().includes(query));
}

/**
 * Filters by free text and normalized state, then applies the visible sort.
 * Raw systemd states stay searchable so operators can find "activating" units.
 */
export function filterAndSortServices(
  services: ServiceUnit[],
  query: string,
  stateFilter: ServiceStateFilter,
  sort: ServiceSort,
) {
  const normalizedQuery = query.trim().toLowerCase();
  const direction = sort.direction === "asc" ? 1 : -1;

  return services
    .filter(
      (service) =>
        (stateFilter === "all" || service.state === stateFilter) &&
        matchesQuery(service, normalizedQuery),
    )
    .sort((left, right) => compareServices(left, right, sort) * direction);
}

/**
 * Builds the copyable transcript shown in the terminal-like output panel.
 * Command, exit status, stdout, and stderr stay together instead of being
 * scattered across separate cards.
 */
export function serviceActionTranscript(result: ServiceActionResult) {
  const exitStatus =
    result.exitCode === null ? "no exit status" : `exit ${result.exitCode.toString()}`;
  const stdout = result.stdout.trimEnd();
  const stderr = result.stderr.trimEnd();

  return [
    `$ ${result.command}`,
    `${exitStatus} - ${result.durationMs.toString()} ms`,
    "",
    "stdout:",
    stdout.length > 0 ? stdout : "No stdout.",
    "",
    "stderr:",
    stderr.length > 0 ? stderr : "No stderr.",
  ].join("\n");
}
