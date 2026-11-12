import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { buildLatestJson, generateLatestJson, resolveArtifactPaths } from "./latest-json.mjs";

const fixtures = resolve(process.cwd(), "scripts/fixtures/latest-json");
const assets = join(fixtures, "release-assets");
const script = resolve(process.cwd(), "scripts/latest-json.mjs");
const temporaryDirectories = [];

function temporaryDirectory() {
  const directory = mkdtempSync(join(tmpdir(), "serverdesk-latest-json-"));
  temporaryDirectories.push(directory);
  return directory;
}

function fixtureSignature(file, version, includeVersion = true) {
  const trustedComment = includeVersion
    ? `trusted comment: timestamp:1794484800\tfile:${file}\tversion:${version}`
    : `trusted comment: timestamp:1794484800\tfile:${file}`;
  return Buffer.from(
    [
      "untrusted comment: signature from tauri secret key",
      "RkFLRXNpZ25hdHVyZQ==",
      trustedComment,
      "RkFLRXNpZ25hdHVyZQ==",
      "",
    ].join("\n"),
    "utf8",
  ).toString("base64");
}

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

describe("latest.json artifact resolution", () => {
  it("selects the updater AppImage, NSIS installer, and universal macOS archive", () => {
    const paths = resolveArtifactPaths(assets);

    expect(paths["linux-x86_64"]).toMatch(/ServerDesk_0\.1\.1_amd64\.AppImage$/);
    expect(paths["windows-x86_64"]).toMatch(/ServerDesk_0\.1\.1_x64-setup\.exe$/);
    expect(paths["darwin-x86_64"]).toMatch(/ServerDesk_0\.1\.1_universal\.app\.tar\.gz$/);
    expect(paths["darwin-aarch64"]).toBe(paths["darwin-x86_64"]);
  });

  it("supports separate architectures through explicit paths", () => {
    const paths = resolveArtifactPaths(assets, {
      darwinX86_64: "ServerDesk_0.1.1_universal.app.tar.gz",
      darwinAarch64: "ServerDesk_0.1.1_universal.app.tar.gz",
    });

    expect(paths["darwin-x86_64"]).toBe(paths["darwin-aarch64"]);
  });

  it("prefers a compressed AppImage when the bundler emits one", () => {
    const directory = temporaryDirectory();
    for (const file of readdirSync(assets)) {
      copyFileSync(join(assets, file), join(directory, file));
    }
    writeFileSync(join(directory, "ServerDesk_0.1.1_amd64.AppImage.tar.gz"), "archive");
    writeFileSync(join(directory, "ServerDesk_0.1.1_amd64.AppImage.tar.gz.sig"), "archive-sig");

    const paths = resolveArtifactPaths(directory);
    expect(paths["linux-x86_64"]).toMatch(/\.AppImage\.tar\.gz$/);
  });

  it("fails when a signature file is missing", () => {
    const directory = temporaryDirectory();
    for (const file of readdirSync(assets)) {
      if (file !== "ServerDesk_0.1.1_amd64.AppImage.sig") {
        copyFileSync(join(assets, file), join(directory, file));
      }
    }

    expect(() =>
      generateLatestJson(
        {
          version: "0.1.1",
          "pub-date": "2026-11-12T12:00:00Z",
          "asset-dir": directory,
          output: join(directory, "latest.json"),
        },
        directory,
      ),
    ).toThrow(/missing signature file/);
  });
});

describe("latest.json manifest", () => {
  it("builds a deterministic manifest for every supported platform", () => {
    const artifacts = {
      "linux-x86_64": { url: "https://example.test/linux.tar.gz", signature: "linux-sig" },
      "windows-x86_64": { url: "https://example.test/win.exe", signature: "win-sig" },
      "darwin-x86_64": { url: "https://example.test/mac.tar.gz", signature: "mac-sig" },
      "darwin-aarch64": { url: "https://example.test/mac.tar.gz", signature: "mac-sig" },
    };
    const input = {
      version: "0.1.1",
      notes: "Release notes",
      pubDate: "2026-11-12T12:00:00Z",
      baseUrl: "https://example.test/releases/download/v0.1.1",
      artifacts,
    };

    const first = buildLatestJson(input);
    const second = buildLatestJson(input);

    expect(first).toEqual({
      version: "0.1.1",
      notes: "Release notes",
      pub_date: "2026-11-12T12:00:00Z",
      platforms: {
        "darwin-aarch64": artifacts["darwin-aarch64"],
        "darwin-x86_64": artifacts["darwin-x86_64"],
        "linux-x86_64": artifacts["linux-x86_64"],
        "windows-x86_64": artifacts["windows-x86_64"],
      },
    });
    expect(JSON.stringify(first)).toBe(JSON.stringify(second));
    expect(Object.keys(first.platforms)).toEqual([
      "darwin-aarch64",
      "darwin-x86_64",
      "linux-x86_64",
      "windows-x86_64",
    ]);
  });

  it("rejects incomplete manifests", () => {
    expect(() =>
      buildLatestJson({
        version: "0.1.1",
        notes: "",
        pubDate: "2026-11-12T12:00:00Z",
        baseUrl: "https://example.test/v0.1.1",
        artifacts: {
          "linux-x86_64": { url: "https://example.test/linux", signature: "" },
          "windows-x86_64": { url: "https://example.test/win", signature: "sig" },
          "darwin-x86_64": { url: "https://example.test/mac", signature: "sig" },
          "darwin-aarch64": { url: "https://example.test/mac", signature: "sig" },
        },
      }),
    ).toThrow(/signature for linux-x86_64 is empty/);
  });

  it("rejects invalid versions and publication dates", () => {
    const artifacts = {
      "linux-x86_64": { url: "https://example.test/linux", signature: "sig" },
      "windows-x86_64": { url: "https://example.test/win", signature: "sig" },
      "darwin-x86_64": { url: "https://example.test/mac", signature: "sig" },
      "darwin-aarch64": { url: "https://example.test/mac", signature: "sig" },
    };

    expect(() =>
      buildLatestJson({
        version: "latest",
        notes: "",
        pubDate: "2026-11-12T12:00:00Z",
        baseUrl: "https://example.test/v0.1.1",
        artifacts,
      }),
    ).toThrow(/invalid version/);

    expect(() =>
      buildLatestJson({
        version: "0.1.1",
        notes: "",
        pubDate: "not-a-date",
        baseUrl: "https://example.test/v0.1.1",
        artifacts,
      }),
    ).toThrow(/invalid publication date/);
  });

  it("writes a manifest through the CLI", () => {
    const outputDirectory = temporaryDirectory();
    const output = join(outputDirectory, "latest.json");

    execFileSync(
      process.execPath,
      [
        script,
        "--version",
        "0.1.1",
        "--notes-file",
        join(fixtures, "release-notes.txt"),
        "--pub-date",
        "2026-11-12T12:00:00Z",
        "--asset-dir",
        assets,
        "--output",
        output,
      ],
      { stdio: "pipe" },
    );

    const manifest = JSON.parse(readFileSync(output, "utf8"));
    expect(manifest.version).toBe("0.1.1");
    expect(manifest.notes).toContain("Added signed in-app updates");
    expect(manifest.pub_date).toBe("2026-11-12T12:00:00Z");
    expect(manifest.platforms["linux-x86_64"].url).toContain("ServerDesk_0.1.1_amd64.AppImage");
    const signature = Buffer.from(manifest.platforms["linux-x86_64"].signature, "base64").toString(
      "utf8",
    );
    expect(signature).toContain("version:0.1.1");
    expect(manifest.platforms["windows-x86_64"].url).toContain("ServerDesk_0.1.1_x64-setup.exe");
  });

  it("rejects a signature for a different version", () => {
    const directory = temporaryDirectory();
    for (const file of readdirSync(assets)) {
      copyFileSync(join(assets, file), join(directory, file));
    }
    writeFileSync(
      join(directory, "ServerDesk_0.1.1_amd64.AppImage.sig"),
      fixtureSignature("ServerDesk_0.1.1_amd64.AppImage", "0.0.9"),
    );

    expect(() =>
      generateLatestJson(
        {
          version: "0.1.1",
          "pub-date": "2026-11-12T12:00:00Z",
          "asset-dir": directory,
          output: join(directory, "latest.json"),
        },
        directory,
      ),
    ).toThrow(/is for version 0.0.9, expected 0.1.1/);
  });

  it("rejects a signature without a signed version", () => {
    const directory = temporaryDirectory();
    for (const file of readdirSync(assets)) {
      copyFileSync(join(assets, file), join(directory, file));
    }
    writeFileSync(
      join(directory, "ServerDesk_0.1.1_amd64.AppImage.sig"),
      fixtureSignature("ServerDesk_0.1.1_amd64.AppImage", "0.1.1", false),
    );

    expect(() =>
      generateLatestJson(
        {
          version: "0.1.1",
          "pub-date": "2026-11-12T12:00:00Z",
          "asset-dir": directory,
          output: join(directory, "latest.json"),
        },
        directory,
      ),
    ).toThrow(/does not record a signed version/);
  });

  it("uses SOURCE_DATE_EPOCH when no explicit date is passed", () => {
    const previous = process.env.SOURCE_DATE_EPOCH;
    process.env.SOURCE_DATE_EPOCH = "1794484800";

    try {
      const manifest = generateLatestJson(
        {
          version: "0.1.1",
          notes: "Notes",
          "asset-dir": assets,
          output: join(temporaryDirectory(), "latest.json"),
        },
        assets,
      );

      expect(manifest.pub_date).toBe("2026-11-12T12:00:00Z");
    } finally {
      if (previous === undefined) {
        delete process.env.SOURCE_DATE_EPOCH;
      } else {
        process.env.SOURCE_DATE_EPOCH = previous;
      }
    }
  });
});
