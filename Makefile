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

release:
	@git cliff --tag v$(VERSION) -o CHANGELOG.md
	@git commit -a -n -m "docs: update CHANGELOG for v$(VERSION)" || true
	@cargo release tag --execute --no-confirm
	@git push origin master
	@git push origin v$(VERSION)

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
	crates/core \
	crates/storage \
	crates/model-router \
	crates/cost \
	crates/resilience \
	crates/compress \
	crates/bridge \
	crates/storage-sqlite

publish-crate:
	@echo "📦 Publishing crates to crates.io..."
	@for crate in $(CRATES_PUBLISH_ORDER); do \
		echo "  Publishing $$crate..."; \
		cargo publish -p $$(basename $$crate) --allow-dirty || exit 1; \
		sleep 2; \
	done
	@echo "✅ All crates published successfully!"

.PHONY: build test fmt clippy lint check-agent-sync release publish-crate update-submodule seed-manifest seed-tag
