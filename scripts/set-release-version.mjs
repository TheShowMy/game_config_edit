import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error("usage: node scripts/set-release-version.mjs X.Y.Z");
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packages = [
  "npm/game-config-edit/package.json",
  "npm/game-config-edit-win32-x64/package.json",
  "npm/game-config-edit-darwin-arm64/package.json",
];

for (const relativePath of packages) {
  const packagePath = path.join(root, relativePath);
  const manifest = JSON.parse(fs.readFileSync(packagePath, "utf8"));
  manifest.version = version;
  if (manifest.name === "game-config-edit") {
    manifest.optionalDependencies["game-config-edit-win32-x64"] = version;
    manifest.optionalDependencies["game-config-edit-darwin-arm64"] = version;
  }
  fs.writeFileSync(packagePath, `${JSON.stringify(manifest, null, 2)}\n`);
}
