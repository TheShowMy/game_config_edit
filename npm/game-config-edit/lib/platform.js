import path from "node:path";

const PLATFORMS = new Map([
  ["win32-x64", "game-config-edit-win32-x64"],
  ["darwin-arm64", "game-config-edit-darwin-arm64"],
]);

export function packageFor(platform, arch) {
  const key = `${platform}-${arch}`;
  const packageName = PLATFORMS.get(key);
  if (!packageName) {
    throw new Error(
      `unsupported platform ${key}; supported platforms: win32-x64, darwin-arm64`,
    );
  }
  return packageName;
}

export function executableFor(packageRoot, platform, arch) {
  packageFor(platform, arch);
  if (platform === "win32") {
    return path.join(packageRoot, "bin", "gconf.exe");
  }
  return path.join(packageRoot, "bin", "gconf");
}
