import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import { executableFor, packageFor } from "../lib/platform.js";

test("selects the Windows x64 native package", () => {
  assert.equal(packageFor("win32", "x64"), "game-config-edit-win32-x64");
});

test("selects the macOS Apple Silicon native package", () => {
  assert.equal(packageFor("darwin", "arm64"), "game-config-edit-darwin-arm64");
});

test("rejects unsupported platforms before launching", () => {
  assert.throws(
    () => packageFor("linux", "x64"),
    /unsupported platform linux-x64.*win32-x64, darwin-arm64/,
  );
});

test("resolves the executable inside each native package", () => {
  assert.equal(
    executableFor("C:/package", "win32", "x64"),
    path.join("C:/package", "bin", "gconf.exe"),
  );
  assert.equal(
    executableFor("/package", "darwin", "arm64"),
    path.join("/package", "bin", "gconf"),
  );
});
