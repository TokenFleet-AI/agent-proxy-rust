# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

---
## [1.2.0](https://github.com/compare/v1.1.0..v1.2.0) - 2026-07-02

### Bug Fixes

- **(ci)** resolve all CI failures - ([3b94ac4](https://github.com/commit/3b94ac48ac8a05c5a4958e9197eeb3b2f00b31a9)) - baoyx
- suppress clippy duration_suboptimal_units lint for Rust 1.96 compatibility - ([ace6250](https://github.com/commit/ace625083d6faabe996f5893d18546a78d40f6a6)) - baoyx
- handle renamed and new clippy lints for Rust 1.96 compatibility - ([b22c5e2](https://github.com/commit/b22c5e24c2a600e979bca082dca6be0ff4be268f)) - baoyx
- allow duration_suboptimal_units lint in is_tryable_past_cooldown - ([142ac92](https://github.com/commit/142ac92c11266d4d7752126a297fc64244d7b382)) - baoyx
- allow duration_suboptimal_units lint in resilience test modules - ([3517454](https://github.com/commit/35174542f33fbf9cfa219cc86c1f9cd97bce5cc5)) - baoyx
- add version to workspace dependencies for crates.io publishing - ([c0e68f4](https://github.com/commit/c0e68f46d91c178eee0c91796a19c72f2640900d)) - baoyx
- add reverse mapping OpenaiChat→OpenaiResponses for streaming - ([3245b82](https://github.com/commit/3245b827b33842a11083f97edb1910d9635bc9fd)) - baoyx
- TokenFleet CN use openai_responses protocol, restrict mimo to openai_chat - ([0253336](https://github.com/commit/0253336e7a8711f21dba9aee76329fccf2e8b82a)) - baoyx

### Documentation

- add release guide with two-step workflow - ([d302f0b](https://github.com/commit/d302f0b826e24c3f6e4ef85d3501eda7cdb81d28)) - baoyx

### Features

- split release into push and publish steps for CI safety - ([5b5306a](https://github.com/commit/5b5306a19ef22d70264ed759ef907164ae017886)) - baoyx
- integrate exception-collector for error reporting (#1) - ([7892915](https://github.com/commit/7892915033cec08a9837b3c2c723ca1d206102ac)) - baoyx

### Miscellaneous Chores

- upgrade Rust toolchain to 1.96.0 and fix clippy lints - ([115a876](https://github.com/commit/115a876dab0b267ee262adc7fc4df048490af015)) - baoyx
- release v1.1.1 - ([cc36905](https://github.com/commit/cc369056788be78abc8317dd914251ded4a8b7c7)) - baoyx
- add bump-version Makefile target - ([b86bfe2](https://github.com/commit/b86bfe295e44d3d5681314961fa131418ca5e3d8)) - baoyx
- add automated release script and document release process - ([3ac42df](https://github.com/commit/3ac42df6d8fbdcf70f36f2377c2ccee3e82ebbcb)) - baoyx
- add debug logging for request tracing - ([96d5fc1](https://github.com/commit/96d5fc1c73b19ecf3a1c487b8bcc14a2c0a45bf8)) - baoyx
- align Makefile with llm-bridge-rust - ([fb0b600](https://github.com/commit/fb0b600e6b9930268571e6f9e86efb3e00a3e54a)) - baoyx
- use exception-collector v0.1 from crates.io - ([efd6fb5](https://github.com/commit/efd6fb5325dba092b38f821225295dff60ef38d3)) - baoyx
- release v1.2.0 - ([324a176](https://github.com/commit/324a176d5c66ce7d4984f217a38635663d4dbea0)) - baoyx

### Refactoring

- use git dependency for exception-collector - ([6080447](https://github.com/commit/6080447c1a865f2c766c385e2efa124e4c078c2e)) - baoyx
- change release flow to bump-version-after-publish - ([09fa099](https://github.com/commit/09fa09906b9534a1af7feaf37d9fdde488db2593)) - baoyx

---
## [1.1.0](https://github.com/compare/v1.0.1..v1.1.0) - 2026-06-24

### Bug Fixes

- security hardening, API consistency, and quality improvements - ([f314aa8](https://github.com/commit/f314aa83d1053de34e49ca7975cb17798f3eccf2)) - baoyx

### Features

- real health probe with cheapest model, 429 30s retry - ([8c26ce2](https://github.com/commit/8c26ce237156d882c284234b98988ba142a73ac4)) - baoyx

### Miscellaneous Chores

- release v1.1.0 - ([c26f264](https://github.com/commit/c26f264cd3cadc6041ff41e579d11aa4855c9b9e)) - baoyx
- sync lockfile and bump tokenless-schema to 1.2 - ([1916fcf](https://github.com/commit/1916fcf87e9edda129590217cfda140a4a7606c0)) - baoyx

---
## [1.0.1](https://github.com/compare/v1.0.0..v1.0.1) - 2026-06-12

### Documentation

- comprehensive documentation overhaul - ([18b54f5](https://github.com/commit/18b54f5b7d011346dea4c7e5ea2f1e15997f575c)) - baoyx
- update CHANGELOG for v1.0.1 - ([8c42dda](https://github.com/commit/8c42dda4412c32cc4eb8dd0a00694ae6762b5fe7)) - baoyx

### Miscellaneous Chores

- **(release)** configure cargo-release with single workspace tag - ([2d7d24a](https://github.com/commit/2d7d24a2e91f70ec6d442c146d01977fd24114d2)) - baoyx

---
## [1.0.0](https://github.com/compare/seed-v1..v1.0.0) - 2026-06-12

### Bug Fixes

- **(cost)** populate rtk_saved_tokens and response_saved_tokens from tokenless reports - ([57854c6](https://github.com/commit/57854c6ce3f1dfd3a25eb4bdfa19c66ad1c2ab46)) - baoyx

### Documentation

- finalize CHANGELOG for v1.0.0 - ([f5d18ef](https://github.com/commit/f5d18ef5348835959925b09b941bfdd4f9a4a094)) - baoyx

### Features

- **(admin)** add compress toggle API endpoints - ([b7c4d47](https://github.com/commit/b7c4d4729686fd29c29837101b434646e044d2f1)) - baoyx
- **(compress)** add enabled toggle to CompressMiddleware - ([73bdf90](https://github.com/commit/73bdf906a9332023fef23f090c8329cd7332c5b7)) - baoyx
- channel ArcSwap hot-reload, cost time fix, model grouping fix, Hourly trend, /admin/projects, /admin/health channels - ([8ae973a](https://github.com/commit/8ae973a4074eddb4e218522988dbe8b0d971f542)) - baoyx

### Miscellaneous Chores

- **(deps)** switch llm-bridge-core to crates.io 0.2.6 - ([63c63af](https://github.com/commit/63c63af84196629d09afecafbe531819686b3097)) - baoyx
- **(seed)** update seed data - ([ed00417](https://github.com/commit/ed00417386180e08ccdf6c59f24add5dac3c2a61)) - baoyx
- bump to 1.0.0, remove attestation, fix deps - ([9f16b69](https://github.com/commit/9f16b694bcbc4b58d878b638e8b2e259570307b7)) - baoyx

### Other

- add overseas models (Anthropic/OpenAI) and reorder TokenFleet channels first - ([39cac5d](https://github.com/commit/39cac5dbe13e96b7dd94c820fad7f7ac490a2f78)) - baoyx
- Update CHANGELOG.md - ([5955427](https://github.com/commit/59554272f2feef72a3a0bb49ecddbef9a354e34f)) - baoyx

### Performance

- downgrade verbose request logs from info to debug - ([642f0c3](https://github.com/commit/642f0c30f7e88961a86d04a5902792fb6186b8a0)) - baoyx

### Refactoring

- **(core)** incremental tokenless report reading with DashMap cursor - ([c972e16](https://github.com/commit/c972e16aa49a56bed88a175e9c9ff7e31608d446)) - baoyx

---
## [seed-v1] - 2026-06-05

### Documentation

- add actual GitHub URLs to Related Projects in 0001 - ([bb0dc40](https://github.com/commit/bb0dc402654d1d184d58b86ef3be2de17c1ecc3c)) - baoyx
- fix llm-bridge-rust branch URL main→master - ([e6af4bc](https://github.com/commit/e6af4bc3f3a3651bb3556aa17705d5cbf7a106ea)) - baoyx
- rewrite README, add Chinese version, switch to Apache-2.0 - ([03140e8](https://github.com/commit/03140e84db84fda6ec9409a0892fa8717f88eb95)) - baoyx
- add user guide, update specs status and index - ([96a8de8](https://github.com/commit/96a8de88e961a231445f26ba40817be92de980b4)) - baoyx
- remove dead agent-proxy-pricing links, clarify pricing is embedded - ([e629345](https://github.com/commit/e629345be44022f82f6abe6e78a90488cbb10928)) - baoyx

### Features

- implement proxy core + all middleware crates + 65 model mappings - ([fe64ea0](https://github.com/commit/fe64ea0791127475b6a1ce66edd7e8577dd1863a)) - baoyx
- implement quota consumption tracking for FlatFee channels - ([6e5f83f](https://github.com/commit/6e5f83f23d3b592da7eb8604a2d370d787fa627a)) - baoyx
- wire admin API, cost UUID, SQLite migrations v2-v4, server integration - ([78982fb](https://github.com/commit/78982fb212d02a84b67e9dd546f316ffeb9c1e9a)) - baoyx
- add resilience crate, compression stats tracking, admin auth, billing migrations - ([52eaf33](https://github.com/commit/52eaf33389c2236ecb7466a6535884b4185d7c91)) - baoyx
- remove CLI crate, consolidate migrations, enhance admin/model-router/storage - ([5dfacff](https://github.com/commit/5dfacff56df32e9b05f08d9b5ea6b3cbe68504ea)) - baoyx
- add tiered pricing, SSE streaming bridge, and PerUnit billing support - ([15e4a04](https://github.com/commit/15e4a0451f1c5963fd225eb4a8a752dca6119bf9)) - baoyx
- remote seed data update mechanism (Phase 1+2) - ([7c45dd1](https://github.com/commit/7c45dd1f8dc6f1a9ececf3f3b39520cf6cf3059a)) - baoyx

### Miscellaneous Chores

- update Cargo.toml authors and repo URLs - ([4a51128](https://github.com/commit/4a51128f914d4c4486ab640fc95b1e80069ffff6)) - baoyx

<!-- generated by git-cliff -->
