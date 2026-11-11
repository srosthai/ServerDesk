import { describe, expect, it } from "vitest";
import {
  databaseProfileFormFromProfile,
  databaseProfileInputFromForm,
  defaultPortForEngine,
  emptyDatabaseProfileForm,
  formatDatabaseEndpoint,
  formatLatency,
  validateDatabaseProfileForm,
} from "@/features/databases/databaseProfileForm";
import type { DatabaseProfile } from "@/types/contracts";

const profile: DatabaseProfile = {
  id: "profile-1",
  serverId: "server-1",
  name: "Primary",
  engine: "postgresql",
  host: "127.0.0.1",
  port: 5432,
  socket: null,
  databaseName: "app",
  username: "app",
  hasCredential: true,
  readOnly: true,
  sslMode: "required",
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: null,
};

function validForm() {
  return {
    ...emptyDatabaseProfileForm("server-1", "postgresql"),
    name: "Primary",
    host: "127.0.0.1",
    port: "5432",
    username: "app",
    password: "fixture-password",
  };
}

describe("database profile form", () => {
  it("uses engine defaults and never loads a saved password into the edit form", () => {
    expect(defaultPortForEngine("mysql")).toBe("3306");
    expect(defaultPortForEngine("postgresql")).toBe("5432");

    const form = databaseProfileFormFromProfile(profile, "server-1");
    expect(form.password).toBe("");
    expect(form.name).toBe("Primary");
    expect(form.sslMode).toBe("required");
    expect(form.readOnly).toBe(true);
  });

  it("rejects missing and unsafe connection values", () => {
    const errors = validateDatabaseProfileForm({
      ...validForm(),
      name: " ",
      serverId: "",
      host: "db internal",
      port: "70000",
      socket: "relative.sock",
      databaseName: "bad\u0000name",
      username: "app user",
    });

    expect(errors.name).toBe("Database profile name is required.");
    expect(errors.serverId).toBe("Select a server profile.");
    expect(errors.host).toBe("Enter a valid database hostname or IP address.");
    expect(errors.port).toBe("Database port must be between 1 and 65535.");
    expect(errors.socket).toBe("The database socket must be an absolute path beginning with /.");
    expect(errors.databaseName).toBe("The database name contains invalid characters.");
    expect(errors.username).toBe("Enter a valid database username without spaces.");
  });

  it("accepts socket profiles and normalizes optional values for the command", () => {
    const form = {
      ...validForm(),
      name: "  Primary  ",
      socket: " /var/run/postgresql/.s.PGSQL.5432 ",
      databaseName: "  ",
      sslMode: "default" as const,
      readOnly: true,
    };

    expect(validateDatabaseProfileForm(form)).toEqual({});
    expect(databaseProfileInputFromForm(form)).toEqual({
      serverId: "server-1",
      name: "Primary",
      engine: "postgresql",
      host: "127.0.0.1",
      port: 5432,
      socket: "/var/run/postgresql/.s.PGSQL.5432",
      databaseName: null,
      username: "app",
      readOnly: true,
      sslMode: null,
      password: "fixture-password",
    });
  });

  it("sends a null password when the edit field is left blank", () => {
    const input = databaseProfileInputFromForm({ ...validForm(), password: "" });

    expect(input.password).toBeNull();
  });

  it("formats endpoints and latency for the connection list", () => {
    expect(formatDatabaseEndpoint("127.0.0.1", 5432)).toBe("127.0.0.1:5432");
    expect(formatDatabaseEndpoint("2001:db8::1", 3306)).toBe("[2001:db8::1]:3306");
    expect(formatDatabaseEndpoint("127.0.0.1", 5432, "/var/run/db.sock")).toBe("/var/run/db.sock");
    expect(formatLatency(3)).toBe("3 ms");
    expect(formatLatency(1200)).toBe("1.2 s");
    expect(formatLatency(null)).toBe("Not measured");
  });
});
