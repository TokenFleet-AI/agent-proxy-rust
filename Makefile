.PHONY: build check test fmt clippy lint ci doc check-agent-sync release release-push release-publish bump publish-crate publish-selected ci-status seed-manifest seed-tag

build:
	cargo build

check:
	cargo check --all-features

test:
	cargo nextest run --all-features

fmt:
	cargo +nightly fmt

clippy:
	cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic

lint: fmt clippy

ci: fmt clippy test

doc:
	cargo doc --open

check-agent-sync:
	@test -f CLAUDE.md || { \
		echo "CLAUDE.md is required for project-level agent instructions."; \
		exit 1; \
	}

# ── Release ──────────────────────────────────────────────────────────
# Full release (三步):
#   make release                    → tag + CHANGELOG + push（不修改版本号）
#   make release-publish            → crates.io 发布全部
#   make bump VERSION=patch|minor   → 发布成功后 bump 版本号
#
# 选择性发布:
#   make ci-status                  → 检查 GitHub CI 状态
#   make publish-selected CRATES="agent-proxy-rust-storage-sqlite" [VERSION=patch]
#   make publish-selected CRATES="agent-proxy-rust-storage agent-proxy-rust-storage-sqlite"

CURRENT_VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

release: release-push ## Step 1: 用当前版本号打 tag + 生成 CHANGELOG + 推送
	@echo ""
	@echo "==> ✅ Step 1 完成: tag v$(CURRENT_VERSION) 已推送"
	@echo "==> 请等待 GitHub Actions CI 通过"
	@echo "==> 查看 CI 状态: gh run list --limit 1"
	@echo "==> CI 通过后执行: make release-publish"
	@echo "==> 发布成功后执行: make bump VERSION=patch|minor"

release-push: ## Step 1: 生成 CHANGELOG、创建 tag（不修改版本号）、推送
	@echo "📦 准备发布 v$(CURRENT_VERSION)..."
	@git cliff --tag "v$(CURRENT_VERSION)" -o CHANGELOG.md
	@git commit -a -n -m "chore: update CHANGELOG for v$(CURRENT_VERSION)" || true
	@git tag -a "v$(CURRENT_VERSION)" -m "Release v$(CURRENT_VERSION)"
	@git push origin master --tags
	@echo "✅ tag v$(CURRENT_VERSION) 已创建并推送"

release-publish: ## Step 2: 发布到 crates.io（CI 通过后执行）
	@$(MAKE) publish-crate

bump: ## Step 3: 发布成功后升级版本号（Usage: make bump VERSION=patch|minor|major）
ifndef VERSION
	$(error Usage: make bump VERSION=patch|minor|major)
endif
	@cargo release version $(VERSION) --execute --workspace --no-confirm
	@cargo release commit --execute --no-confirm
	@git push origin master
	@echo "✅ 版本号已升级并推送"

# ── Crates.io Publishing ────────────────────────────────────────────
# Publish library crates to crates.io in dependency order
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
		cargo release publish --execute -p $$crate --no-confirm || exit 1; \
		sleep 2; \
	done
	@echo "✅ All crates published successfully"

ci-status: ## 检查 GitHub Actions CI 状态
	@echo "🔍 Checking GitHub Actions CI status..."
	@gh run list --limit 1 --json status,conclusion,name,createdAt,headBranch \
		--jq '.[0] | "  \(.name) (\(.headBranch)): \(.status) \(.conclusion // "in progress") \(.createdAt)"'
	@echo ""
	@if gh run list --limit 1 --json status,conclusion --jq '.[0].conclusion' | grep -q '"success"'; then \
		echo "✅ CI passed"; \
	else \
		echo "❌ CI not passing — check with: gh run list --limit 3"; \
		exit 1; \
	fi

publish-selected: ci-status ## 选择性发布 crate（Usage: make publish-selected CRATES="crate1 crate2" [VERSION=patch|minor|major]）
ifndef CRATES
	$(error Usage: make publish-selected CRATES="crate1 crate2 ..." [VERSION=patch|minor|major])
endif
ifdef VERSION
	@echo "⬆️  Bumping version ($(VERSION))..."
	@cargo release version $(VERSION) --execute --workspace --no-confirm
	@cargo release commit --execute --no-confirm
	@echo ""
endif
	@echo "📦 Publishing selected crates..."
	@for crate in $(CRATES); do \
		echo "  Publishing $$crate..."; \
		cargo release publish --execute -p $$crate --no-confirm || exit 1; \
		sleep 2; \
	done
	@echo ""
	@echo "🏷️  Tagging and pushing..."
	@NEW_VER=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/'); \
	git tag -a "v$$NEW_VER" -m "Release v$$NEW_VER" --force; \
	git push origin master --tags
	@echo ""
	@echo "✅ Published: $(CRATES)"

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
