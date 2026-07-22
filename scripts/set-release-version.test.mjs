import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const packageDirectories = [
  "game-config-edit",
  "game-config-edit-win32-x64",
  "game-config-edit-darwin-arm64",
];

function createFixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gconf-release-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, "scripts"), { recursive: true });
  fs.copyFileSync(
    path.join(repositoryRoot, "scripts", "set-release-version.mjs"),
    path.join(root, "scripts", "set-release-version.mjs"),
  );

  for (const directory of packageDirectories) {
    const destination = path.join(root, "npm", directory);
    fs.mkdirSync(destination, { recursive: true });
    fs.copyFileSync(
      path.join(repositoryRoot, "npm", directory, "package.json"),
      path.join(destination, "package.json"),
    );
  }
  return root;
}

function runVersionScript(root, version) {
  return spawnSync(
    process.execPath,
    [path.join(root, "scripts", "set-release-version.mjs"), version],
    { cwd: root, encoding: "utf8" },
  );
}

test("sets every npm package and exact optional dependency version", (t) => {
  const root = createFixture(t);
  const result = runVersionScript(root, "1.2.3");
  assert.equal(result.status, 0, result.stderr);

  const manifests = Object.fromEntries(
    packageDirectories.map((directory) => [
      directory,
      JSON.parse(
        fs.readFileSync(path.join(root, "npm", directory, "package.json"), "utf8"),
      ),
    ]),
  );
  for (const manifest of Object.values(manifests)) {
    assert.equal(manifest.version, "1.2.3");
  }
  assert.deepEqual(manifests["game-config-edit"].optionalDependencies, {
    "game-config-edit-darwin-arm64": "1.2.3",
    "game-config-edit-win32-x64": "1.2.3",
  });
});

for (const version of ["1.2", "1.2.3.4", "1.2.3-beta.1"]) {
  test(`rejects non-release version ${version}`, (t) => {
    const root = createFixture(t);
    const result = runVersionScript(root, version);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /X\.Y\.Z/);
  });
}
