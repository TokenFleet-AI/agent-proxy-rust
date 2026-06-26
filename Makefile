build:
	@cargo build

test:
	@cargo nextest run --all-features

fmt:
	@cargo +nightly fmt -- --check

clippy:
	@cargo clippy --all-targets --all-features -- -D warnings

lint: fmt clippy

check-agent-sync:
	@test -f CLAUDE.md || { \
		echo "CLAUDE.md is required for project-level agent instructions."; \
		exit 1; \
	}

VERSION := $(shell grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

bump-version:
	@if [ -z "$(NEW_VERSION)" ]; then \
		echo "❌ Usage: make bump-version NEW_VERSION=x.y.z"; \
		exit 1; \
	fi
	@echo "📦 Bumping version to $(NEW_VERSION)..."
	@sed -i '' 's/^version = "[^"]*"/version = "$(NEW_VERSION)"/' Cargo.toml
	@# Update workspace crate dependencies in Cargo.toml
	@for crate in core storage model-router cost storage-sqlite bridge compress; do \
		sed -i '' "s/agent-proxy-rust-$$crate = { path = \"[^\"]*\", version = \"[^\"]*\" }/agent-proxy-rust-$$crate = { path = \"crates/$$crate\", version = \"$(NEW_VERSION)\" }/g" Cargo.toml; \
	done
	@# Update internal dependencies in crate Cargo.toml files
	@for crate in crates/compress crates/cost crates/model-router crates/resilience crates/storage-sqlite crates/bridge; do \
		sed -i '' "s/agent-proxy-rust-\(.*\) = { path = \"[^\"]*\", version = \"[^\"]*\" }/agent-proxy-rust-\1 = { path = \"..\/\2\", version = \"$(NEW_VERSION)\" }/g" $$crate/Cargo.toml; \
	done
	@cargo update --workspace 2>&1 | tail -5
	@echo "✅ Version bumped to $(NEW_VERSION)"

release: release-push ## Usage: make release VERSION=x.y.z (step 1: push + tag)
	@echo ""
	@echo "==> Step 1 完成: 代码已推送并创建 tag"
	@echo "==> 请等待 GitHub Actions CI 通过"
	@echo "==> 查看 CI 状态: gh run list --limit 1"
	@echo "==> CI 通过后执行: make release-publish"

release-push: ## Step 1: 生成 CHANGELOG、创建 tag、推送
	@git cliff --tag v$(VERSION) -o CHANGELOG.md
	@git commit -a -n -m "docs: update CHANGELOG for v$(VERSION)" || true
	@cargo release tag --execute --no-confirm
	@git push origin master
	@git push origin v$(VERSION)

release-publish: ## Step 2: 发布到 crates.io（CI 通过后执行）
	@$(MAKE) publish-crate

update-submodule:
	@git submodule update --init --recursive --remote

# ── Seed Data ────────────────────────────────────────────────────────

SEED_DIR := crates/storage-sqlite/seed

seed-manifest:
	@new_ver=$$(jq '.version + 1' $(SEED_DIR)/seed-manifest.json); \
	now=$$(date -u +"%Y-%m-%dT%H:%M:%SZ"); \
	jq -n \
	  --argjson version "$$new_ver" \
	  --arg updatedAt "$$now" \
	  --arg prov_hash "$$(shasum -a 256 $(SEED_DIR)/providers.json | awk '{print $$1}')" \
	  --arg model_hash "$$(shasum -a 256 $(SEED_DIR)/models.json | awk '{print $$1}')" \
	  --arg chan_hash "$$(shasum -a 256 $(SEED_DIR)/channels.json | awk '{print $$1}')" \
	  --arg mm_hash "$$(shasum -a 256 $(SEED_DIR)/model_mappings.json | awk '{print $$1}')" \
	'{ \
	  "version": $$version, \
	  "minSchemaVersion": 1, \
	  "updatedAt": $$updatedAt, \
	  "entries": { \
	    "providers":      {"file": "providers.json",      "sha256": $$prov_hash}, \
	    "models":         {"file": "models.json",         "sha256": $$model_hash}, \
	    "channels":       {"file": "channels.json",       "sha256": $$chan_hash}, \
	    "modelMappings":  {"file": "model_mappings.json", "sha256": $$mm_hash} \
	  } \
	}' > $(SEED_DIR)/seed-manifest.json; \
	echo "✅ seed-manifest.json updated to v$$new_ver"; \
	echo "   providers:      $$prov_hash"; \
	echo "   models:         $$model_hash"; \
	echo "   channels:       $$chan_hash"; \
	echo "   modelMappings:  $$mm_hash"

seed-tag: seed-manifest
	@ver=$$(jq -r '.version' $(SEED_DIR)/seed-manifest.json); \
	tag="seed-v$$ver"; \
	git add $(SEED_DIR)/; \
	git commit -m "chore: bump seed data to v$$ver" || true; \
	git tag -a "$$tag" -m "Seed data v$$ver — providers/models/channels/model_mappings"; \
	echo "✅ Created tag $$tag"; \
	echo "   Run: git push origin $$tag"

# ── Crates.io Publishing ────────────────────────────────────────────
# Publish crates to crates.io in dependency order
# Only library crates are published (not apps/server)

CRATES_PUBLISH_ORDER = \
	agent-proxy-rust-core \
	agent-proxy-rust-storage \
	agent-proxy-rust-model-router \
	agent-proxy-rust-cost \
	agent-proxy-rust-resilience \
	agent-proxy-rust-compress \
	agent-proxy-rust-bridge \
	agent-proxy-rust-storage-sqlite

publish-crate:
	@echo "📦 Publishing crates to crates.io..."
	@for crate in $(CRATES_PUBLISH_ORDER); do \
		echo "  Publishing $$crate..."; \
		cargo publish -p $$crate --allow-dirty || exit 1; \
		sleep 2; \
	done
	@echo "✅ All crates published successful… (truncated)

.PHONY: build test fmt clippy lint check-agent-sync release release-push release-publish publish-crate update-submodule seed-manifest seed-tag
