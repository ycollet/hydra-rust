# Releasing installers

Install the version-editing command once:

```bash
cargo install cargo-edit --locked
```

Update the desktop app's version, commit it, and push a matching tag. For
example, to release `0.2.0`:

```bash
cargo set-version --package hydra-rust 0.2.0
git add Cargo.toml Cargo.lock
git commit -m "Bump version to 0.2.0"
git push origin HEAD

git tag v0.2.0
git push origin v0.2.0
```

Pushing the tag triggers [Release installers](workflows/release.yml). Once all
builds succeed, the workflow publishes these installers and `SHA256SUMS` to
the tag's GitHub Release:

- Debian `.deb` (x86-64)
- Fedora `.rpm` (x86-64)
- macOS `.dmg` (separate Intel and Apple Silicon builds)
- Windows setup `.exe` (x86-64)

Installer versions come from `[package].version` in the root `Cargo.toml`,
not the tag name. Use a new version and tag for each release. Cargo prerelease
versions, such as `0.2.0-beta.1`, produce prerelease GitHub Releases.

The installers package the desktop app with default Cargo features, not the
VST3/CLAP plugin. Windows installers are unsigned, and macOS apps are ad-hoc
signed but not notarized.
