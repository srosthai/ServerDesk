import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

function fail(message) {
  console.error(`Version check failed: ${message}`);
  process.exit(1);
}

function readJson(relativePath) {
  try {
    return JSON.parse(readFileSync(join(repositoryRoot, relativePath), "utf8"));
  } catch (error) {
    fail(`could not read ${relativePath}: ${error.message}`);
  }
}

const packageMetadata = readJson("package.json");
const tauriConfig = readJson("src-tauri/tauri.conf.json");
const cargoManifest = readFileSync(join(repositoryRoot, "src-tauri/Cargo.toml"), "utf8");

const cargoVersion = cargoManifest.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!cargoVersion) {
  fail("could not find the package version in src-tauri/Cargo.toml");
}

const versions = {
  "package.json": packageMetadata.version,
  "src-tauri/Cargo.toml": cargoVersion,
  "src-tauri/tauri.conf.json": tauriConfig.version,
};

for (const [file, version] of Object.entries(versions)) {
  if (version !== packageMetadata.version) {
    fail(`${file} has version ${version}, expected ${packageMetadata.version}`);
  }
}

if (tauriConfig.productName !== "ServerDesk") {
  fail(`unexpected product name ${JSON.stringify(tauriConfig.productName)}`);
}

if (tauriConfig.identifier !== "com.serverdesk.app") {
  fail(`unexpected bundle identifier ${JSON.stringify(tauriConfig.identifier)}`);
}

const iconFiles = tauriConfig.bundle?.icon ?? [];
if (iconFiles.length === 0) {
  fail("the Tauri bundle does not list any icon files");
}

for (const iconFile of iconFiles) {
  const path = join(repositoryRoot, "src-tauri", iconFile);
  if (!existsSync(path) || statSync(path).size === 0) {
    fail(`bundle icon ${iconFile} is missing or empty`);
  }
}

function readPngSize(relativePath) {
  const path = join(repositoryRoot, relativePath);
  const contents = readFileSync(path);
  const isPng = contents.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
  if (!isPng) {
    fail(`${relativePath} is not a PNG file`);
  }

  return {
    width: contents.readUInt32BE(16),
    height: contents.readUInt32BE(20),
  };
}

const expectedPngSizes = {
  "src-tauri/icons/32x32.png": 32,
  "src-tauri/icons/64x64.png": 64,
  "src-tauri/icons/128x128.png": 128,
  "src-tauri/icons/128x128@2x.png": 256,
  "src-tauri/icons/icon.png": 512,
};

for (const [file, expectedSize] of Object.entries(expectedPngSizes)) {
  const { width, height } = readPngSize(file);
  if (width !== expectedSize || height !== expectedSize) {
    fail(`${file} is ${width}x${height}, expected ${expectedSize}x${expectedSize}`);
  }
}

for (const brandAsset of [
  "assets/brand/serverdesk-logo.png",
  "assets/brand/serverdesk-mark-source.png",
  "src/assets/brand/serverdesk-mark.png",
  "src-tauri/icons/icon-source.png",
]) {
  const path = join(repositoryRoot, brandAsset);
  if (!existsSync(path) || statSync(path).size === 0) {
    fail(`brand asset ${brandAsset} is missing or empty`);
  }
}

const tagArgumentIndex = process.argv.indexOf("--tag");
if (tagArgumentIndex !== -1) {
  const tag = process.argv[tagArgumentIndex + 1];
  if (tag !== `v${packageMetadata.version}`) {
    fail(`release tag ${tag} does not match version v${packageMetadata.version}`);
  }
}

console.log(
  `Version metadata is consistent at ${packageMetadata.version} (ServerDesk, com.serverdesk.app).`,
);
