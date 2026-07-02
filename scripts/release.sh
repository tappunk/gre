#!/bin/bash
set -euo pipefail

BIN_NAME="gre"
TARGET_ARCH="macos-arm64"
RUST_TARGET="aarch64-apple-darwin"
GITHUB_REPO="tappunk/gre"

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=true
  shift
fi

BUMP="${1:-patch}"
NOTES="${2:-}"

echo "[PROC] Verifying deployment dependencies..."
for tool in cargo gh shasum tar awk git sed; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "[ERR] Required CLI tool '$tool' is missing."
    exit 1
  fi
done

if ! gh auth status >/dev/null 2>&1; then
  echo "[ERR] GitHub CLI is not authenticated. Run 'gh auth login'."
  exit 1
fi

if [[ ! "$BUMP" =~ ^(patch|minor|major)$ ]]; then
  echo "[ERR] Invalid bump type '$BUMP'. Use: patch, minor, or major"
  exit 1
fi

if [[ -n $(git ls-files --others --exclude-standard) ]]; then
  echo "[ERR] Untracked files found in workspace. Add or remove them before releasing."
  git ls-files --others --exclude-standard
  exit 1
fi

if [[ -n $(git status --porcelain) ]]; then
  echo "[ERR] Uncommitted changes detected. Stash or commit before releasing."
  exit 1
fi

if [[ $(git branch --show-current) != "main" ]]; then
  echo "[ERR] You must be on the 'main' branch to cut a release."
  exit 1
fi

git fetch origin
if [[ -n $(git log HEAD..origin/main --oneline) ]]; then
  echo "[ERR] Local 'main' is behind 'origin/main'. Pull latest changes first."
  exit 1
fi

echo "[PROC] Executing strict code quality gates..."
cargo fmt --check || {
  echo "[ERR] Code formatting violations found. Run 'cargo fmt'."
  exit 1
}
cargo clippy -- -D warnings || {
  echo "[ERR] Clippy warnings detected. Fix them before releasing."
  exit 1
}
cargo test || {
  echo "[ERR] Test suite execution failed."
  exit 1
}

cargo publish --dry-run || {
  echo "[ERR] Cargo publish dry run failed. Fix packaging before releasing."
  exit 1
}

CURRENT_VERSION=$(grep -m 1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
if [[ -z "$CURRENT_VERSION" ]]; then
  echo "[ERR] Could not read current version from Cargo.toml"
  exit 1
fi

IFS='.' read -r MAJOR MINOR PATCH <<<"$CURRENT_VERSION"
case "$BUMP" in
patch) PATCH=$((PATCH + 1)) ;;
minor)
  MINOR=$((MINOR + 1))
  PATCH=0
  ;;
major)
  MAJOR=$((MAJOR + 1))
  MINOR=0
  PATCH=0
  ;;
esac
NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"

echo "Preparing Apple Silicon Release: v$CURRENT_VERSION -> v$NEW_VERSION ($BUMP)"
if $DRY_RUN; then
  echo "[INFO] Dry run complete. Code is pristine and ready for release."
  exit 0
fi

BACKUP_CARGO_TOML=""
rollback() {
  echo ""
  echo "[CRIT] Release pipeline interrupted!"
  if git rev-parse "v$NEW_VERSION" >/dev/null 2>&1; then
    git tag -d "v$NEW_VERSION"
  fi
  if [[ -n "$BACKUP_CARGO_TOML" && -f "$BACKUP_CARGO_TOML" ]]; then
    cp "$BACKUP_CARGO_TOML" Cargo.toml
  fi
  rm -f Cargo.toml.bak "${ARCHIVE_NAME:-}" "${CHECKSUM_NAME:-}" "$BACKUP_CARGO_TOML" 2>/dev/null || true
  if [[ -n "${STAGING_DIR:-}" && -d "${STAGING_DIR}" ]]; then
    rm -rf "${STAGING_DIR}"
  fi
  echo "[WARN] Rolled back. Re-run scripts/release.sh to try again."
}
trap rollback ERR

echo "[PROC] Updating versioning configuration..."
BACKUP_CARGO_TOML=$(mktemp)
cp Cargo.toml "$BACKUP_CARGO_TOML"
sed -i.bak "s/version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" Cargo.toml
rm Cargo.toml.bak

cargo update -p "$BIN_NAME"

echo "[PROC] Compiling optimized release binary for Apple Silicon..."
cargo build --release --target "$RUST_TARGET"

echo "[PROC] Packaging distribution archives..."
ARCHIVE_NAME="${BIN_NAME}-${NEW_VERSION}-bin-${TARGET_ARCH}.tar.gz"
CHECKSUM_NAME="${ARCHIVE_NAME}.sha256"
STAGING_DIR="$(mktemp -d)"

mkdir -p "${STAGING_DIR}/${BIN_NAME}"
cp "target/${RUST_TARGET}/release/${BIN_NAME}" "${STAGING_DIR}/${BIN_NAME}/"
cp README.md LICENSE "${STAGING_DIR}/${BIN_NAME}/" 2>/dev/null || true

tar -czf "$ARCHIVE_NAME" -C "$STAGING_DIR" "${BIN_NAME}"
shasum -a 256 "$ARCHIVE_NAME" >"$CHECKSUM_NAME"
rm -rf "$STAGING_DIR"
rm -f "$BACKUP_CARGO_TOML"
BACKUP_CARGO_TOML=""

echo "[PROC] Recording version changes to Git history..."
git add Cargo.toml Cargo.lock
git commit -m "chore: release v$NEW_VERSION [skip ci]"
git tag -a "v$NEW_VERSION" -m "Release v$NEW_VERSION"

trap - ERR

echo "[PROC] Synchronizing changes with remote origin..."
git push origin main
git push origin "v$NEW_VERSION"

echo "[PROC] Waiting for GitHub mirror to expose tag v$NEW_VERSION..."
MAX_TAG_WAIT_ATTEMPTS=30
TAG_WAIT_INTERVAL_SECS=2
for ((attempt = 1; attempt <= MAX_TAG_WAIT_ATTEMPTS; attempt++)); do
  if gh api "repos/${GITHUB_REPO}/git/ref/tags/v${NEW_VERSION}" >/dev/null 2>&1; then
    echo "[PROC] GitHub mirror now contains tag v$NEW_VERSION"
    break
  fi

  if [[ "$attempt" -eq "$MAX_TAG_WAIT_ATTEMPTS" ]]; then
    echo "[ERR] Timed out waiting for GitHub mirror tag v$NEW_VERSION in ${GITHUB_REPO}"
    echo "[ERR] Confirm mirror health or push tag directly to GitHub, then re-run release."
    exit 1
  fi

  sleep "$TAG_WAIT_INTERVAL_SECS"
done

echo "[PROC] Deploying GitHub Release and assets..."
if [[ -n "$NOTES" ]]; then
  gh release create "v$NEW_VERSION" "$ARCHIVE_NAME" "$CHECKSUM_NAME" \
    -R "$GITHUB_REPO" \
    --title "v$NEW_VERSION" \
    --notes "$NOTES"
else
  gh release create "v$NEW_VERSION" "$ARCHIVE_NAME" "$CHECKSUM_NAME" \
    -R "$GITHUB_REPO" \
    --title "v$NEW_VERSION" \
    --generate-notes
fi

echo "[PROC] Propagating release configuration to Homebrew tap..."
RAW_SHA=$(awk '{print $1}' "${CHECKSUM_NAME}")
TAP_DIR="$(mktemp -d)"

git clone --depth 1 "git@github.com:tappunk/homebrew-gre.git" "$TAP_DIR"

cat <<EOF >"${TAP_DIR}/Formula/gre.rb"
class Gre < Formula
  desc "Super fast git recap for multiple repositories"
  homepage "https://github.com/tappunk/gre"
  version "${NEW_VERSION}"

  depends_on arch: :arm64
  depends_on :macos

  url "https://github.com/tappunk/gre/releases/download/v#{version}/gre-#{version}-bin-macos-arm64.tar.gz"
  sha256 "${RAW_SHA}"

  def install
    bin.install "gre"
  end

  test do
    (testpath/"config.toml").write <<~EOS
      paths = ["#{testpath}"]
    EOS
    output = shell_output("#{bin}/gre --config #{testpath}/config.toml --json")
    assert_match '"schema_version": "1"', output
    assert_match '"summary"', output
    assert_match '"repos"', output
  end
end
EOF

(
  cd "$TAP_DIR"
  git add Formula/gre.rb
  git commit -m "bump: gre v${NEW_VERSION}"
  git push origin main
)
rm -rf "$TAP_DIR"

echo "[PROC] Cleaning up local packaging assets..."
rm -f "$ARCHIVE_NAME" "$CHECKSUM_NAME"

echo "[PROC] Publishing crate package to crates.io..."
cargo publish

echo "[ SUCCESS ] Release v$NEW_VERSION fully deployed!"
