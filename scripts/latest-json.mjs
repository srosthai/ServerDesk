import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { basename, isAbsolute, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const DEFAULT_REPOSITORY = "srosthai/ServerDesk";
export const PLATFORM_ORDER = ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"];

const versionPattern = /^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

function normalizeVersion(value) {
  const version = String(value ?? "").trim();
  if (!versionPattern.test(version)) {
    throw new Error(`invalid version ${JSON.stringify(value)}; expected semver such as 0.1.1`);
  }
  return version.replace(/^v/, "");
}

function normalizePubDate(value) {
  const raw = String(value ?? "").trim();
  if (!raw) {
    throw new Error("a publication date is required; pass --pub-date or set SOURCE_DATE_EPOCH");
  }

  const date = /^\d+$/.test(raw) ? new Date(Number(raw) * 1000) : new Date(raw);
  if (Number.isNaN(date.getTime())) {
    throw new Error(`invalid publication date ${JSON.stringify(value)}`);
  }

  return date.toISOString().replace(/\.\d{3}Z$/, "Z");
}

function defaultBaseUrl(version, repository) {
  return `https://github.com/${repository}/releases/download/v${version}`;
}

function normalizeBaseUrl(value, version, repository = DEFAULT_REPOSITORY) {
  const raw = String(value ?? "").trim() || defaultBaseUrl(version, repository);
  let parsed;
  try {
    parsed = new URL(raw.endsWith("/") ? raw.slice(0, -1) : raw);
  } catch {
    throw new Error(`invalid base URL ${JSON.stringify(raw)}`);
  }

  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
    throw new Error("the base URL must use http or https");
  }

  return parsed.toString().replace(/\/$/, "");
}

function buildUrl(baseUrl, fileName) {
  return `${baseUrl}/${encodeURIComponent(fileName)}`;
}

function readSignature(signaturePath, artifactPath) {
  if (!existsSync(signaturePath) || !statSync(signaturePath).isFile()) {
    throw new Error(`missing signature file for ${basename(artifactPath)}: ${signaturePath}`);
  }

  const signature = readFileSync(signaturePath, "utf8").trim();
  if (!signature) {
    throw new Error(`signature file is empty: ${signaturePath}`);
  }

  return signature;
}

function assertSignedVersion(signature, version, artifactPath) {
  let decoded;
  try {
    decoded = Buffer.from(signature, "base64").toString("utf8");
  } catch {
    throw new Error(`signature for ${basename(artifactPath)} is not valid base64`);
  }

  if (!decoded.includes("trusted comment:")) {
    throw new Error(`signature for ${basename(artifactPath)} is not a minisign signature`);
  }

  const match = /(?:^|[\t\s])version:v?([0-9A-Za-z.+-]+)(?:$|[\t\s])/m.exec(decoded);
  if (!match) {
    throw new Error(
      `signature for ${basename(artifactPath)} does not record a signed version; re-sign with a current Tauri CLI`,
    );
  }
  if (match[1] !== version) {
    throw new Error(
      `signature for ${basename(artifactPath)} is for version ${match[1]}, expected ${version}`,
    );
  }
}

function matchingFiles(files, pattern) {
  return files.filter((file) => pattern.test(file));
}

function selectSingle(candidates, label) {
  if (candidates.length === 0) {
    return null;
  }
  if (candidates.length > 1) {
    throw new Error(
      `multiple ${label} artifacts found (${candidates.join(", ")}); pass an explicit platform path`,
    );
  }

  return candidates[0];
}

function selectArtifact(files, patterns, label) {
  for (const pattern of patterns) {
    const selected = selectSingle(matchingFiles(files, pattern), label);
    if (selected) {
      return selected;
    }
  }

  return null;
}

function resolveOverride(assetDirectory, value) {
  const path = isAbsolute(value) ? value : resolve(assetDirectory, value);
  if (!existsSync(path) || !statSync(path).isFile()) {
    throw new Error(`artifact does not exist: ${path}`);
  }

  return path;
}

/**
 * Resolves release artifacts from a flat release directory. Linux prefers the
 * compressed AppImage when Tauri emits one and otherwise uses the signed
 * AppImage directly; Windows uses the NSIS installer; macOS uses the universal
 * app archive for both architectures unless explicit paths are supplied.
 */
export function resolveArtifactPaths(assetDirectory, overrides = {}) {
  if (!existsSync(assetDirectory) || !statSync(assetDirectory).isDirectory()) {
    throw new Error(`release asset directory does not exist: ${assetDirectory}`);
  }

  const files = readdirSync(assetDirectory)
    .filter((file) => statSync(join(assetDirectory, file)).isFile())
    .sort();

  const linux =
    overrides.linux ??
    selectArtifact(files, [/\.AppImage\.tar\.gz$/, /\.AppImage$/], "Linux AppImage");
  const windows =
    overrides.windows ?? selectArtifact(files, [/-setup\.exe$/], "Windows NSIS installer");
  const darwin =
    overrides.darwin ?? selectArtifact(files, [/\.app\.tar\.gz$/], "macOS app archive");
  const darwinX86 = overrides.darwinX86_64 ?? darwin;
  const darwinArm = overrides.darwinAarch64 ?? darwin;

  const resolved = {
    "linux-x86_64": linux,
    "windows-x86_64": windows,
    "darwin-x86_64": darwinX86,
    "darwin-aarch64": darwinArm,
  };

  for (const [platform, value] of Object.entries(resolved)) {
    if (!value) {
      throw new Error(`could not find an updater artifact for ${platform}`);
    }
    resolved[platform] = resolveOverride(assetDirectory, value);
  }

  return resolved;
}

/**
 * Builds a deterministic latest.json manifest. Platform keys are emitted in a
 * fixed order, and no timestamps or random values are generated here.
 */
export function buildLatestJson({ version, notes, pubDate, baseUrl, artifacts }) {
  const normalizedVersion = normalizeVersion(version);
  const normalizedDate = normalizePubDate(pubDate);
  if (baseUrl !== undefined) {
    normalizeBaseUrl(baseUrl, normalizedVersion);
  }
  const platforms = {};

  for (const platform of PLATFORM_ORDER) {
    const entry = artifacts?.[platform];
    if (!entry || typeof entry.url !== "string" || typeof entry.signature !== "string") {
      throw new Error(`missing URL or signature for platform ${platform}`);
    }
    if (!entry.signature.trim()) {
      throw new Error(`the signature for ${platform} is empty`);
    }

    let parsedUrl;
    try {
      parsedUrl = new URL(entry.url);
    } catch {
      throw new Error(`invalid download URL for ${platform}: ${entry.url}`);
    }
    if (parsedUrl.protocol !== "https:" && parsedUrl.protocol !== "http:") {
      throw new Error(`download URL for ${platform} must use http or https`);
    }

    platforms[platform] = {
      url: entry.url,
      signature: entry.signature.trim(),
    };
  }

  return {
    version: normalizedVersion,
    notes: String(notes ?? ""),
    pub_date: normalizedDate,
    platforms,
  };
}

function parseArguments(argv) {
  const options = {};
  const valueFlags = new Set([
    "--version",
    "--notes",
    "--notes-file",
    "--pub-date",
    "--asset-dir",
    "--output",
    "--base-url",
    "--repository",
    "--linux-x86_64",
    "--windows-x86_64",
    "--darwin",
    "--darwin-x86_64",
    "--darwin-aarch64",
  ]);

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") {
      options.help = true;
      continue;
    }

    const [flag, inlineValue] = argument.includes("=")
      ? argument.split(/=(.*)/s, 2)
      : [argument, undefined];
    if (!valueFlags.has(flag)) {
      throw new Error(`unknown argument ${argument}`);
    }

    const value = inlineValue ?? argv[index + 1];
    if (value === undefined) {
      throw new Error(`missing value for ${flag}`);
    }
    if (inlineValue === undefined) {
      index += 1;
    }
    options[flag.slice(2)] = value;
  }

  return options;
}

function usage() {
  return `Usage:
  node scripts/latest-json.mjs --version 0.1.1 --pub-date 2026-11-12T12:00:00Z \\
    --asset-dir release-assets --output release-assets/latest.json [options]

Options:
  --version <semver>          Release version (required)
  --pub-date <RFC3339|epoch>  Deterministic publication date (or SOURCE_DATE_EPOCH)
  --asset-dir <dir>           Flat directory of release artifacts
  --output <file>             Output latest.json path
  --notes <text>              Release notes text
  --notes-file <file>         Read release notes from a file
  --base-url <url>            Asset URL base (default: GitHub release tag URL)
  --repository <owner/repo>   GitHub repository for the default base URL
  --linux-x86_64 <file>       Override the Linux AppImage artifact
  --windows-x86_64 <file>     Override the Windows NSIS artifact
  --darwin <file>             Use one macOS artifact for both architectures
  --darwin-x86_64 <file>      Override the Intel macOS artifact
  --darwin-aarch64 <file>     Override the Apple silicon macOS artifact`;
}

export function generateLatestJson(options, assetDirectory) {
  const version = normalizeVersion(options.version);
  const repository = options.repository || DEFAULT_REPOSITORY;
  const baseUrl = normalizeBaseUrl(options["base-url"], version, repository);
  const notesFile = options["notes-file"];
  const notes = notesFile ? readFileSync(notesFile, "utf8").trim() : options.notes;
  const paths = resolveArtifactPaths(assetDirectory, {
    linux: options["linux-x86_64"],
    windows: options["windows-x86_64"],
    darwin: options.darwin,
    darwinX86_64: options["darwin-x86_64"],
    darwinAarch64: options["darwin-aarch64"],
  });

  const artifacts = {};
  for (const platform of PLATFORM_ORDER) {
    const path = paths[platform];
    const signature = readSignature(`${path}.sig`, path);
    assertSignedVersion(signature, version, path);
    artifacts[platform] = {
      url: buildUrl(baseUrl, basename(path)),
      signature,
    };
  }

  const pubDate = options["pub-date"] ?? process.env.SOURCE_DATE_EPOCH;

  return buildLatestJson({
    version,
    notes:
      notes?.trim() ||
      `See https://github.com/${repository}/releases/tag/v${version} for release notes.`,
    pubDate,
    baseUrl,
    artifacts,
  });
}

export function writeLatestJson(manifest, outputPath) {
  writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

export function main(argv = process.argv.slice(2)) {
  const options = parseArguments(argv);
  if (options.help) {
    console.log(usage());
    return 0;
  }

  if (!options.version) {
    throw new Error("--version is required");
  }
  if (!options["asset-dir"]) {
    throw new Error("--asset-dir is required");
  }
  if (!options.output) {
    throw new Error("--output is required");
  }

  const assetDirectory = resolve(options["asset-dir"]);
  const outputPath = resolve(options.output);
  const manifest = generateLatestJson(options, assetDirectory);
  writeLatestJson(manifest, outputPath);
  console.log(
    `Wrote ${outputPath} for v${manifest.version} with platforms: ${Object.keys(
      manifest.platforms,
    ).join(", ")}`,
  );
  return 0;
}

const invokedPath = process.argv[1] ? pathToFileURL(process.argv[1]).href : "";
if (invokedPath === import.meta.url) {
  try {
    process.exitCode = main();
  } catch (error) {
    console.error(`latest.json generation failed: ${error.message}`);
    process.exitCode = 1;
  }
}
