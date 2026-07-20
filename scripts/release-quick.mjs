#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, '..');
const args = process.argv.slice(2);
const dryRun = args.includes('--dry-run');

if (args.includes('-h') || args.includes('--help')) {
  console.log('Usage: npm run release:quick -- [patch|minor|major|x.y.z] [--dry-run]');
  process.exit(0);
}

const bumpArg = args.find((arg) => !arg.startsWith('-')) ?? 'patch';
const npmCmd = process.platform === 'win32' ? 'npm.cmd' : 'npm';

const files = {
  packageJson: path.join(repoRoot, 'package.json'),
  cargoToml: path.join(repoRoot, 'src-tauri', 'Cargo.toml'),
  tauriConf: path.join(repoRoot, 'src-tauri', 'tauri.conf.json'),
  cargoLock: path.join(repoRoot, 'src-tauri', 'Cargo.lock'),
};

function readText(filePath) {
  return fs.readFileSync(filePath, 'utf8');
}

function writeText(filePath, content) {
  fs.writeFileSync(filePath, content, 'utf8');
}

function replaceOnce(content, pattern, replacement, label) {
  const next = content.replace(pattern, replacement);
  if (next === content) {
    throw new Error(`Could not update ${label}.`);
  }
  return next;
}

function parseSemver(value) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(value);
  if (!match) {
    throw new Error(`Bad version: ${value}`);
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
  };
}

function bumpVersion(currentVersion, input) {
  if (/^\d+\.\d+\.\d+$/.test(input)) {
    return input;
  }

  const current = parseSemver(currentVersion);
  if (input === 'patch') {
    return `${current.major}.${current.minor}.${current.patch + 1}`;
  }
  if (input === 'minor') {
    return `${current.major}.${current.minor + 1}.0`;
  }
  if (input === 'major') {
    return `${current.major + 1}.0.0`;
  }

  throw new Error(`Unknown bump type: ${input}`);
}

function updateFile(filePath, pattern, replacement, label) {
  const next = replaceOnce(readText(filePath), pattern, replacement, label);
  if (!dryRun) {
    writeText(filePath, next);
  }
}

const packageJson = JSON.parse(readText(files.packageJson));
const nextVersion = bumpVersion(packageJson.version, bumpArg);

if (!dryRun) {
  execFileSync(
    npmCmd,
    ['version', nextVersion, '--no-git-tag-version', '--allow-same-version'],
    { cwd: repoRoot, stdio: 'inherit' },
  );
}

updateFile(
  files.packageJson,
  /"version":\s*"[^"]+"/,
  `"version": "${nextVersion}"`,
  'package.json',
);
updateFile(
  files.cargoToml,
  /(\[package\]\r?\nname = "quota-float"\r?\nversion = ")[^"]+(")/,
  `$1${nextVersion}$2`,
  'src-tauri/Cargo.toml',
);
updateFile(
  files.tauriConf,
  /"version":\s*"[^"]+"/,
  `"version": "${nextVersion}"`,
  'src-tauri/tauri.conf.json',
);
updateFile(
  files.cargoLock,
  /(\[\[package\]\]\r?\nname = "quota-float"\r?\nversion = ")[^"]+(")/,
  `$1${nextVersion}$2`,
  'src-tauri/Cargo.lock',
);

if (!dryRun) {
  const branch = execFileSync('git', ['branch', '--show-current'], {
    cwd: repoRoot,
    encoding: 'utf8',
  }).trim();

  if (!branch) {
    throw new Error('Detached HEAD. Switch to a branch first.');
  }

  execFileSync('git', ['add', 'package.json', 'package-lock.json', 'src-tauri/Cargo.toml', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.lock'], {
    cwd: repoRoot,
    stdio: 'inherit',
  });
  execFileSync('git', ['commit', '-m', `chore(release): v${nextVersion}`], {
    cwd: repoRoot,
    stdio: 'inherit',
  });
  execFileSync('git', ['tag', `v${nextVersion}`], {
    cwd: repoRoot,
    stdio: 'inherit',
  });
  execFileSync('git', ['push', 'origin', branch], {
    cwd: repoRoot,
    stdio: 'inherit',
  });
  execFileSync('git', ['push', 'origin', `v${nextVersion}`], {
    cwd: repoRoot,
    stdio: 'inherit',
  });
}

console.log(`${dryRun ? '[dry-run] ' : ''}ready: v${nextVersion}`);
