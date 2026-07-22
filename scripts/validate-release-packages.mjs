import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error("usage: node scripts/validate-release-packages.mjs X.Y.Z");
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packages = [
  {
    directory: "npm/game-config-edit",
    name: "game-config-edit",
    files: ["README.md", "bin/gconf.js", "lib/platform.js", "package.json"],
  },
  {
    directory: "npm/game-config-edit-win32-x64",
    name: "game-config-edit-win32-x64",
    os: ["win32"],
    cpu: ["x64"],
    files: ["README.md", "bin/gconf.exe", "package.json"],
  },
  {
    directory: "npm/game-config-edit-darwin-arm64",
    name: "game-config-edit-darwin-arm64",
    os: ["darwin"],
    cpu: ["arm64"],
    files: ["README.md", "bin/gconf", "package.json"],
  },
];
const packageReports = [];

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function assertStringArray(actual, expected, message) {
  assert(
    Array.isArray(actual) &&
      JSON.stringify([...actual].sort()) === JSON.stringify([...expected].sort()),
    `${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
  );
}

for (const definition of packages) {
  const packageRoot = path.join(root, definition.directory);
  const packageJsonPath = path.join(packageRoot, "package.json");
  const manifest = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));

  assert(manifest.name === definition.name, `${definition.name}: package name mismatch`);
  assert(
    manifest.version === version,
    `${definition.name}: expected version ${version}, got ${manifest.version}`,
  );
  assert(
    typeof manifest.license === "string" && manifest.license.length > 0,
    `${definition.name}: license is required`,
  );
  assert(
    fs.existsSync(path.join(packageRoot, "README.md")),
    `${definition.name}: README.md is required`,
  );

  if (definition.os) {
    assertStringArray(manifest.os, definition.os, `${definition.name}: invalid os`);
    assertStringArray(manifest.cpu, definition.cpu, `${definition.name}: invalid cpu`);
    if (definition.name === "game-config-edit-darwin-arm64") {
      const executable = path.join(packageRoot, "bin", "gconf");
      if (process.platform !== "win32") {
        assert(
          (fs.statSync(executable).mode & 0o111) !== 0,
          `${definition.name}: bundled gconf must be executable`,
        );
      }
    }
  } else {
    assert(manifest.engines?.node === ">=22", `${definition.name}: Node >=22 is required`);
    assert(manifest.bin?.gconf === "bin/gconf.js", `${definition.name}: invalid gconf bin`);
    assert(
      manifest.optionalDependencies?.["game-config-edit-win32-x64"] === version,
      `${definition.name}: Windows dependency must exactly match ${version}`,
    );
    assert(
      manifest.optionalDependencies?.["game-config-edit-darwin-arm64"] === version,
      `${definition.name}: macOS dependency must exactly match ${version}`,
    );
  }

  const npmArguments = [
    "pack",
    "--json",
    "--dry-run",
    "--workspace=false",
    packageRoot,
  ];
  const packed =
    process.platform === "win32"
      ? spawnSync(
          `npm pack --json --dry-run --workspace=false "${packageRoot}"`,
          { cwd: root, encoding: "utf8", shell: true },
        )
      : spawnSync("npm", npmArguments, { cwd: root, encoding: "utf8" });
  assert(
    packed.status === 0,
    `${definition.name}: npm pack failed\n${packed.error?.message || packed.stderr || packed.stdout}`,
  );
  const result = JSON.parse(packed.stdout)[0];
  const actualFiles = result.files.map((file) => file.path).sort();
  assertStringArray(
    actualFiles,
    definition.files,
    `${definition.name}: package contents do not match the whitelist`,
  );
  packageReports.push({
    name: definition.name,
    packedSize: result.size,
    unpackedSize: result.unpackedSize,
    fileCount: actualFiles.length,
  });
}

console.log(`Validated npm release packages for ${version}`);
for (const report of packageReports) {
  console.log(
    `${report.name}: ${report.packedSize} packed bytes, ` +
      `${report.unpackedSize} unpacked bytes, ${report.fileCount} files`,
  );
}

if (process.env.GITHUB_STEP_SUMMARY) {
  const rows = packageReports.map(
    (report) =>
      `| ${report.name} | ${report.packedSize} | ${report.unpackedSize} | ${report.fileCount} |`,
  );
  fs.appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    [
      `## npm package sizes for ${version}`,
      "",
      "| Package | Packed bytes | Unpacked bytes | Files |",
      "|---|---:|---:|---:|",
      ...rows,
      "",
    ].join("\n"),
  );
}
