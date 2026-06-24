#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Release Script for agent-proxy-rust
# Automates version bumping, changelog, tagging, and crates.io publishing
# ============================================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log() { echo -e "${GREEN}✓${NC} $1"; }
warn() { echo -e "${YELLOW}⚠${NC} $1"; }
err() { echo -e "${RED}✗${NC} $1" >&2; }

# ── Pre-flight checks ───────────────────────────────────────────────────────

if [ -z "${1:-}" ]; then
    echo "Usage: $0 <VERSION>"
    echo "Example: $0 1.2.0"
    exit 1
fi

VERSION="$1"
CURRENT_BRANCH=$(git branch --show-current)

if [ "$CURRENT_BRANCH" != "master" ]; then
    err "Must be on master branch (currently on $CURRENT_BRANCH)"
    exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
    err "Working tree is not clean. Commit or stash changes first."
    exit 1
fi

# Validate version format (semver)
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    err "Invalid version format: $VERSION (expected x.y.z)"
    exit 1
fi

# Check if tag already exists
if git tag -l "v$VERSION" | grep -q .; then
    err "Tag v$VERSION already exists"
    exit 1
fi

log "Starting release v$VERSION"

# ── Step 1: Bump version ────────────────────────────────────────────────────

echo ""
echo "📦 Step 1: Bumping version to $VERSION..."

# Update workspace version
sed -i '' "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" Cargo.toml

# Update workspace dependencies
for crate in core storage model-router cost storage-sqlite bridge compress; do
    sed -i '' "s/agent-proxy-rust-$crate = { path = \"[^\"]*\", version = \"[^\"]*\" }/agent-proxy-rust-$crate = { path = \"crates\/$crate\", version = \"$VERSION\" }/g" Cargo.toml
done

# Update internal dependencies in each crate
for crate in crates/compress crates/cost crates/model-router crates/resilience crates/storage-sqlite crates/bridge; do
    if [ -f "$crate/Cargo.toml" ]; then
        sed -i '' "s/agent-proxy-rust-\([^ ]*\) = { path = \"[^\"]*\", version = \"[^\"]*\" }/agent-proxy-rust-\1 = { path = \"..\/\1\", version = \"$VERSION\" }/g" "$crate/Cargo.toml"
    fi
done

# Sync Cargo.lock
cargo update --workspace 2>&1 | tail -5
log "Version bumped to $VERSION"

# ── Step 2: Generate changelog ──────────────────────────────────────────────

echo ""
echo "📝 Step 2: Generating changelog..."
if command -v git-cliff &> /dev/null; then
    git cliff --tag "v$VERSION" -o CHANGELOG.md
    log "Changelog updated"
else
    warn "git-cliff not found, skipping changelog generation"
    warn "Install with: cargo install git-cliff"
fi

# ── Step 3: Verify build ────────────────────────────────────────────────────

echo ""
echo "🔨 Step 3: Verifying build..."
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3
log "Build verified"

# ── Step 4: Commit and tag ──────────────────────────────────────────────────

echo ""
echo "🏷️  Step 4: Creating commit and tag..."
git add -A
git commit -m "chore: release v$VERSION"
git tag -a "v$VERSION" -m "Release v$VERSION"
log "Created commit and tag v$VERSION"

# ── Step 5: Push ────────────────────────────────────────────────────────────

echo ""
echo "🚀 Step 5: Pushing to remote..."
git push origin master
git push origin "v$VERSION"
log "Pushed to origin/master with tag v$VERSION"

# ── Step 6: Publish to crates.io ────────────────────────────────────────────

echo ""
echo "📦 Step 6: Publishing to crates.io..."

CRATES_PUBLISH_ORDER=(
    "agent-proxy-rust-core"
    "agent-proxy-rust-storage"
    "agent-proxy-rust-model-router"
    "agent-proxy-rust-cost"
    "agent-proxy-rust-resilience"
    "agent-proxy-rust-compress"
    "agent-proxy-rust-bridge"
    "agent-proxy-rust-storage-sqlite"
)

for crate in "${CRATES_PUBLISH_ORDER[@]}"; do
    echo "  Publishing $crate..."
    cargo publish -p "$crate" --allow-dirty
    sleep 2
done
log "All crates published to crates.io"

# ── Done ─────────────────────────────────────────────────────────────────────

echo ""
echo "═══════════════════════════════════════════════════════════════════════════"
echo -e "${GREEN}🎉 Release v$VERSION completed successfully!${NC}"
echo "═══════════════════════════════════════════════════════════════════════════"
echo ""
echo "Published crates:"
for crate in "${CRATES_PUBLISH_ORDER[@]}"; do
    echo "  • $crate v$VERSION"
done
echo ""
echo "GitHub: https://github.com/TokenFleet-AI/agent-proxy-rust/releases/tag/v$VERSION"
echo ""
