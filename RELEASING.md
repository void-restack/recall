# Releasing recall

Every push to `main` runs CI (`cargo fmt --check`, `clippy -D warnings`, `cargo test`).
Releases are cut from a git tag.

## Cut a release

1. Bump `version` in `Cargo.toml` (and `flake.nix`), commit.
2. Tag and push:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
3. The **Release** workflow (`.github/workflows/release.yml`) then:
   - creates a **draft** GitHub Release with generated notes, and
   - builds and attaches binaries + `sha256` checksums for:
     `x86_64`/`aarch64` Linux (gnu + musl x86_64) and `x86_64`/`aarch64` macOS,
     each as `recall-v0.1.0-<target>.tar.gz`.
4. Review the draft release and **publish** it.

## Distribution channels

Prebuilt binaries from the release are the source of truth; the packages below point at them.

### Homebrew
Formula: [`packaging/homebrew/recall.rb`](packaging/homebrew/recall.rb).
1. Create a tap repo `void-restack/homebrew-tap` with the formula at `Formula/recall.rb`.
2. After each release, update `version` and the four `sha256` values (from the release's `*.sha256` assets).
3. Users install with:
   ```bash
   brew install void-restack/tap/recall
   ```

### Nix
The repo is a flake. No release step needed — users run:
```bash
nix run github:void-restack/recall
# or add the flake as an input and use packages.default
```
Bump `version` in `flake.nix` alongside `Cargo.toml`.

### Debian / Ubuntu (.deb)
Config lives in `Cargo.toml` under `[package.metadata.deb]`.
```bash
cargo install cargo-deb
cargo deb            # writes target/debian/recall_<ver>_<arch>.deb
```
Attach the `.deb` to the release or host in an apt repo.

### Fedora / RHEL (.rpm)
Config lives in `Cargo.toml` under `[package.metadata.generate-rpm]`.
```bash
cargo install cargo-generate-rpm
cargo build --release
cargo generate-rpm   # writes target/generate-rpm/recall-<ver>.rpm
```

### Arch (AUR)
Template: [`packaging/aur/PKGBUILD`](packaging/aur/PKGBUILD).
1. Bump `pkgver`, update `sha256sums` for the source tarball.
2. Regenerate `.SRCINFO` (`makepkg --printsrcinfo > .SRCINFO`) and push to the `recall` AUR repo.

## Needs your accounts / one-time setup
- **Homebrew tap** repo (`homebrew-tap`).
- **AUR** account + SSH key to push the `recall` package.
- No crates.io step: the `recall` id is taken, so recall is not published to crates.io.
- The release workflow uses the built-in `GITHUB_TOKEN` — no secrets to configure.
