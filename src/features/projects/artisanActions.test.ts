import { describe, expect, it } from "vitest";
import {
  ARTISAN_ACTIONS,
  artisanActionConfig,
  artisanCommand,
  artisanTranscript,
  shellQuote,
} from "@/features/projects/artisanActions";
import type { ArtisanActionName } from "@/types/contracts";

describe("artisan action helpers", () => {
  it("builds the exact fixed command for every supported action", () => {
    const expected: [ArtisanActionName, string][] = [
      ["about", "cd -- '/var/www/storefront' && php artisan about"],
      ["route-list", "cd -- '/var/www/storefront' && php artisan route:list"],
      ["migrate-status", "cd -- '/var/www/storefront' && php artisan migrate:status"],
      ["queue-restart", "cd -- '/var/www/storefront' && php artisan queue:restart"],
      ["config-cache", "cd -- '/var/www/storefront' && php artisan config:cache"],
      ["config-clear", "cd -- '/var/www/storefront' && php artisan config:clear"],
      ["cache-clear", "cd -- '/var/www/storefront' && php artisan cache:clear"],
      ["optimize-clear", "cd -- '/var/www/storefront' && php artisan optimize:clear"],
    ];

    expect(ARTISAN_ACTIONS.map((action) => action.action)).toEqual(
      expected.map(([action]) => action),
    );
    for (const [action, command] of expected) {
      expect(artisanCommand("/var/www/storefront", action)).toBe(command);
    }
  });

  it("quotes the target path as one shell argument without allowing another command", () => {
    expect(shellQuote("/var/www/storefront")).toBe("'/var/www/storefront'");
    expect(shellQuote("/var/www/My App's Files")).toBe("'/var/www/My App'\\''s Files'");
    expect(artisanCommand("/var/www/My App's Files", "about")).toBe(
      "cd -- '/var/www/My App'\\''s Files' && php artisan about",
    );
  });

  it("marks exactly the high-impact actions as destructive", () => {
    const destructive = ARTISAN_ACTIONS.filter((action) => action.destructive).map(
      (action) => action.action,
    );
    expect(destructive).toEqual([
      "queue-restart",
      "config-cache",
      "config-clear",
      "cache-clear",
      "optimize-clear",
    ]);
    expect(artisanActionConfig["queue-restart"].confirmation).toContain("gracefully");
    expect(artisanActionConfig.about.confirmation).toBeNull();
  });

  it("includes the command, working directory, exit status, and bounded streams in the transcript", () => {
    const transcript = artisanTranscript({
      serverId: "server-1",
      projectId: "project-1",
      action: "route-list",
      command: "cd -- '/var/www/storefront' && php artisan route:list",
      targetPath: "/var/www/storefront",
      stdout: "route output\n",
      stderr: "route failure detail\n",
      exitCode: 3,
      succeeded: false,
      startedAt: 1_762_665_720,
      durationMs: 42,
      secretsRedacted: true,
      outputTruncated: false,
      connectionState: "connected",
    });

    expect(transcript).toContain("php artisan route:list");
    expect(transcript).toContain("Working directory: /var/www/storefront");
    expect(transcript).toContain("Exit code 3");
    expect(transcript).toContain("stdout:\nroute output");
    expect(transcript).toContain("stderr:\nroute failure detail");
  });
});
