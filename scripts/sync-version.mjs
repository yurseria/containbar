#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync, execSync } from "node:child_process";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const checkOnly = process.argv.includes("--check");
const rawVersion = process.env.ARG_0;
let autoVersionHook = false;
let dryRun = false;

function incrementVersion(current, bump) {
  const match = current.replace(/^v/, "").match(/^(\d+)\.(\d+)\.(\d+)$/);
  if (!match) throw new Error(`Cannot increment invalid version: ${current}`);

  let major = Number(match[1]);
  let minor = Number(match[2]);
  let patch = Number(match[3]);
  if (bump === "major") {
    major += 1;
    minor = 0;
    patch = 0;
  } else if (bump === "minor") {
    minor += 1;
    patch = 0;
  } else if (bump === "patch") {
    patch += 1;
  } else {
    throw new Error(`Cannot increment version with bump: ${bump}`);
  }
  return `${major}.${minor}.${patch}`;
}

function resolveVersion() {
  if (rawVersion) {
    try {
      const hook = JSON.parse(rawVersion);
      if (hook && typeof hook === "object") {
        autoVersionHook = true;
        dryRun = Boolean(hook.dryRun);
        if (hook.useVersion) return String(hook.useVersion).replace(/^v/, "");
        const latest = execSync("git describe --tags --abbrev=0", {
          cwd: root,
          encoding: "utf8",
        }).trim();
        return incrementVersion(latest, hook.bump);
      }
    } catch (error) {
      if (rawVersion.trim().startsWith("{")) throw error;
    }
    return rawVersion.replace(/^v/, "");
  }

  return execSync("git describe --tags --abbrev=0", {
    cwd: root,
    encoding: "utf8",
  })
    .trim()
    .replace(/^v/, "");
}

const version = resolveVersion();

if (!version || version === "0.0.0") {
  console.error("No version found");
  process.exit(1);
}

if (dryRun) {
  console.log(`Would synchronize version ${version}`);
  process.exit(0);
}

function updateJson(filepath) {
  const content = JSON.parse(readFileSync(filepath, "utf8"));
  if (checkOnly) {
    if (content.version !== version) {
      throw new Error(`${filepath} has version ${content.version}, expected ${version}`);
    }
    return;
  }
  content.version = version;
  writeFileSync(filepath, JSON.stringify(content, null, 2) + "\n");
}

function updatePackageLock(filepath) {
  const content = JSON.parse(readFileSync(filepath, "utf8"));
  if (checkOnly) {
    if (content.version !== version || content.packages?.[""]?.version !== version) {
      throw new Error(`${filepath} is not synchronized to ${version}`);
    }
    return;
  }
  content.version = version;
  if (content.packages?.[""]) content.packages[""].version = version;
  writeFileSync(filepath, JSON.stringify(content, null, 2) + "\n");
}

function updateToml(filepath) {
  let content = readFileSync(filepath, "utf8");
  const current = content.match(/^(version\s*=\s*")([\d.]+)(".*)/m)?.[2];
  if (checkOnly) {
    if (current !== version) {
      throw new Error(`${filepath} has version ${current}, expected ${version}`);
    }
    return;
  }
  content = content.replace(/^(version\s*=\s*")[\d.]+(".*)/m, `$1${version}$2`);
  writeFileSync(filepath, content);
}

function updateCargoLock(filepath) {
  let content = readFileSync(filepath, "utf8");
  const packagePattern = /(\[\[package\]\]\nname = "containbar"\nversion = ")[^"]+("\n)/;
  const current = content.match(packagePattern)?.[0].match(/version = "([^"]+)"/)?.[1];
  if (checkOnly) {
    if (current !== version) {
      throw new Error(`${filepath} has containbar ${current}, expected ${version}`);
    }
    return;
  }
  content = content.replace(packagePattern, `$1${version}$2`);
  writeFileSync(filepath, content);
}

const targets = [
  resolve(root, "package.json"),
  resolve(root, "package-lock.json"),
  resolve(root, "src-tauri/tauri.conf.json"),
  resolve(root, "src-tauri/Cargo.toml"),
  resolve(root, "src-tauri/Cargo.lock"),
];

for (const t of targets) {
  if (t.endsWith("package-lock.json")) updatePackageLock(t);
  else if (t.endsWith("Cargo.lock")) updateCargoLock(t);
  else if (t.endsWith(".toml")) updateToml(t);
  else updateJson(t);
}

if (!checkOnly) {
  execFileSync("git", ["add", "--", ...targets], { cwd: root, stdio: "inherit" });

  // Auto's git-tag plugin tags the current commit, so a version hook must
  // commit these files before that plugin runs. This keeps the tag, app
  // metadata and package metadata on the exact same version.
  if (autoVersionHook) {
    let hasChanges = false;
    try {
      execFileSync("git", ["diff", "--cached", "--quiet"], {
        cwd: root,
        stdio: "ignore",
      });
    } catch (error) {
      if (error.status !== 1) throw error;
      hasChanges = true;
    }

    if (hasChanges) {
      execFileSync(
        "git",
        ["commit", "-m", `chore: sync version v${version} [skip ci]`],
        { cwd: root, stdio: "inherit" },
      );
    }
  }
}

console.log(`${checkOnly ? "Verified" : "Synced"} version ${version} in all packages`);
