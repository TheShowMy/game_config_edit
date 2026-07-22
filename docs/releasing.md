# npm release

Pushing a three-part numeric tag matching `vX.Y.Z`, such as `v0.1.0`, starts `.github/workflows/release.yml`. The workflow builds and tests the Windows x64 and macOS Apple Silicon applications, signs and notarizes the macOS application, validates the exact npm package contents, then publishes these npm packages in order:

1. `game-config-edit-win32-x64`
2. `game-config-edit-darwin-arm64`
3. `game-config-edit`

The Cargo package version must match the tag without its `v` prefix. The workflow sets all npm package versions and the main package's optional dependency versions from the tag.

## Publish a version

1. Update `version` in `Cargo.toml`, then run `cargo check` so `Cargo.lock` records the same package version.
2. Commit and push the release change.
3. Create and push the tag:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

Publishing is retry-safe only for the same Git commit: rerunning the workflow skips a package version when its npm `gitHead` matches the tag commit. If the same version already belongs to another commit, the workflow fails and requires a new version tag. Successful runs add the packed and unpacked package sizes, all three package links, integrity values and SHA-1 sums to the GitHub Actions job summary.

## GitHub configuration

The release workflow expects these GitHub Actions secrets:

- `NPM_TOKEN`
- `APPLE_CERTIFICATE_BASE64`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_APP_PASSWORD`

The following non-password values may be configured as GitHub Actions variables or secrets:

- `APPLE_SIGNING_IDENTITY`
- `APPLE_ID`
- `APPLE_TEAM_ID`

The macOS job must run on an Apple Silicon runner. It signs, notarizes, and staples the app before either native npm package is published.
