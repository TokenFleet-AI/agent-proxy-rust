#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# ── Config ──────────────────────────────────────────────────────────
PROXY_KEY="${AGENT_PROXY_API_KEY:-sk-proxy-dev-key}"
ADMIN_KEY="${AGENT_PROXY_ADMIN_KEY:-admin-dev-key}"
DB_DIR="${HOME}/.tokenfleet-ai"
DB_PATH="${DB_DIR}/agent-proxy.db"
LISTEN="${AGENT_PROXY_LISTEN:-127.0.0.1:11837}"

# ── Ensure DB directory exists ──────────────────────────────────────
mkdir -p "${DB_DIR}"

# ── Build ───────────────────────────────────────────────────────────
echo "[start] Building agent-proxy (release)..."
cargo build --release --manifest-path "${PROJECT_DIR}/Cargo.toml"

echo ""
echo "═══════════════════════════════════════════════════════════"
echo "  Proxy Key:   ${PROXY_KEY}"
echo "  Admin Key:   ${ADMIN_KEY}"
echo "  Listen:      ${LISTEN}"
echo "  DB:          ${DB_PATH}"
echo "═══════════════════════════════════════════════════════════"
echo ""
echo "  Set channel key:"
echo "  curl -X PUT ${LISTEN}/admin/channels/deepseek/api-key \\"
echo "    -H 'Content-Type: application/json' \\"
echo "    -H 'x-admin-key: ${ADMIN_KEY}' \\"
echo "    -d '{\"apiKey\":\"sk-your-deepseek-key\"}'"
echo ""

AGENT_PROXY_API_KEY="${PROXY_KEY}" \
AGENT_PROXY_ADMIN_KEY="${ADMIN_KEY}" \
    "${PROJECT_DIR}/target/release/agent-proxy-rust-server" \
    --db-path "${DB_PATH}"
