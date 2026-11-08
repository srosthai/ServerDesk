import { describe, expect, it } from "vitest";
import {
  filterAndSortServices,
  serviceActionTranscript,
  type ServiceSort,
} from "@/features/services/serviceView";
import type { ServiceActionResult, ServiceUnit } from "@/types/contracts";

const services: ServiceUnit[] = [
  {
    name: "nginx.service",
    state: "active",
    activeState: "active",
    subState: "running",
    loadState: "loaded",
    description: "A high performance web server",
  },
  {
    name: "apache2.service",
    state: "failed",
    activeState: "failed",
    subState: "failed",
    loadState: "loaded",
    description: "The Apache HTTP Server",
  },
  {
    name: "mysql.service",
    state: "inactive",
    activeState: "inactive",
    subState: "dead",
    loadState: "loaded",
    description: "MySQL Community Server",
  },
  {
    name: "redis-server.service",
    state: "unknown",
    activeState: "activating",
    subState: "start",
    loadState: "loaded",
    description: "Redis persistent key-value database",
  },
  {
    name: "php8.3-fpm.service",
    state: "active",
    activeState: "active",
    subState: "running",
    loadState: "loaded",
    description: "The PHP 8.3 FastCGI Process Manager",
  },
];

const nameAsc: ServiceSort = { key: "name", direction: "asc" };

describe("service view helpers", () => {
  it("filters by free text across names, descriptions, and raw states", () => {
    expect(filterAndSortServices(services, "php", "all", nameAsc).map((unit) => unit.name)).toEqual(
      ["php8.3-fpm.service"],
    );
    expect(
      filterAndSortServices(services, "activating", "all", nameAsc).map((unit) => unit.name),
    ).toEqual(["redis-server.service"]);
    expect(
      filterAndSortServices(services, "database", "all", nameAsc).map((unit) => unit.name),
    ).toEqual(["redis-server.service"]);
  });

  it("filters by normalized state without relying on color", () => {
    expect(filterAndSortServices(services, "", "failed", nameAsc).map((unit) => unit.name)).toEqual(
      ["apache2.service"],
    );
    expect(
      filterAndSortServices(services, "", "unknown", nameAsc).map((unit) => unit.name),
    ).toEqual(["redis-server.service"]);
  });

  it("sorts names naturally and states by operational severity", () => {
    const names = filterAndSortServices(services, "", "all", nameAsc).map((unit) => unit.name);
    expect(names).toEqual([
      "apache2.service",
      "mysql.service",
      "nginx.service",
      "php8.3-fpm.service",
      "redis-server.service",
    ]);

    const byState = filterAndSortServices(services, "", "all", {
      key: "state",
      direction: "asc",
    }).map((unit) => unit.name);
    expect(byState).toEqual([
      "apache2.service",
      "redis-server.service",
      "nginx.service",
      "php8.3-fpm.service",
      "mysql.service",
    ]);

    const reversed = filterAndSortServices(services, "", "all", {
      key: "state",
      direction: "desc",
    }).map((unit) => unit.name);
    expect(reversed[0]).toBe("mysql.service");
    expect(reversed.at(-1)).toBe("apache2.service");
  });

  it("builds a copyable command transcript with exit status, stdout, and stderr", () => {
    const result: ServiceActionResult = {
      serverId: "server-1",
      unit: "nginx.service",
      action: "restart",
      command: "/usr/bin/sudo -n -- /usr/bin/systemctl restart -- nginx.service",
      succeeded: true,
      stdout: "restarted\n",
      stderr: "",
      exitCode: 0,
      durationMs: 42,
      permissionDenied: false,
      sudoGuidance: null,
      verifiedStatus: null,
      stateVerified: true,
      verificationError: null,
      error: null,
      connectionState: "connected",
    };

    const transcript = serviceActionTranscript(result);
    expect(transcript).toContain(
      "$ /usr/bin/sudo -n -- /usr/bin/systemctl restart -- nginx.service",
    );
    expect(transcript).toContain("exit 0 - 42 ms");
    expect(transcript).toContain("stdout:\nrestarted");
    expect(transcript).toContain("stderr:\nNo stderr.");

    const failed = serviceActionTranscript({
      ...result,
      succeeded: false,
      stdout: "",
      stderr: "sudo: a password is required",
      exitCode: 1,
    });
    expect(failed).toContain("exit 1");
    expect(failed).toContain("stdout:\nNo stdout.");
    expect(failed).toContain("stderr:\nsudo: a password is required");
  });
});
