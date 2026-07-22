#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";
import process from "node:process";

import { executableFor, packageFor } from "../lib/platform.js";

const require = createRequire(import.meta.url);

function fail(message) {
  process.stderr.write(`gconf: ${message}\n`);
  process.exit(1);
}

let packageName;
try {
  packageName = packageFor(process.platform, process.arch);
} catch (error) {
  fail(error.message);
}

let packageRoot;
try {
  packageRoot = path.dirname(require.resolve(`${packageName}/package.json`));
} catch {
  fail(
    `native package ${packageName} is missing for ${process.platform}-${process.arch}; ` +
      "reinstall with: npm install -g game-config-edit",
  );
}

const executable = executableFor(packageRoot, process.platform, process.arch);
const result = spawnSync(executable, process.argv.slice(2), {
  stdio: "inherit",
  windowsHide: false,
});

if (result.error) {
  fail(`failed to start native application: ${result.error.message}`);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
