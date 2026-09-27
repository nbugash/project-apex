# Running and looking at the application.
#
# This project is developed on a headless remote box, so "see the GUI" needs two different
# answers: run it on a machine that has a display, or photograph it on one that does not.

SHELL := /bin/bash
SHOT ?= reports/app.png

# Which directory `make local` opens as the workspace. Anything git knows about: the default
# is this repository, so a first run has real branches, real changes and a real history to look
# at rather than an empty fixture.
WS ?= $(CURDIR)
# Where a local run keeps its session and its cache. Separate from a real profile so that
# experimenting -- opening an odd workspace, corrupting state on purpose -- costs nothing.
LOCAL_PROFILE ?= .local-profile

.DEFAULT_GOAL := help

.PHONY: help setup dev run local local-shot local-reset shot test gate clean next verify pipeline no-network

help: ## Show this help
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | awk -F':.*?## ' '{printf "  \033[1m%-10s\033[0m %s\n", $$1, $$2}'
	@echo
	@echo "  Run it with a workspace open: make local   (WS=/path/to/a/repo)"
	@echo "  On a machine with a display:  make dev"
	@echo "  On this headless box:         make local-shot   (writes $(SHOT))"
	@echo "  Where the backlog stands:     make next   (add F=F005 for one feature)"

setup: ## Install dependencies (run once after cloning)
	npm ci

dev: ## Run the app with hot reload — needs a display
	npm run ds:sync
	cargo build -p apex-engine
	npm run tauri dev

run: ## Build and run the app once — needs a display
	npm run build
	cargo build --workspace
	./target/debug/apex-shell

local: ## Run a local copy with an engine and a workspace already open (WS=/path/to/repo)
	@# Three things a bare `make dev` does not do, each of which leaves the app looking
	@# broken rather than unconfigured.
	@#
	@# 1. APEX_LOCAL_ENGINE. Without it `engine_target()` finds no engine, every workspace
	@#    call answers Offline, and the tree, the editor and git are all empty -- which is
	@#    indistinguishable from a real outage, because it *is* the outage path.
	@# 2. APEX_OPEN_WORKSPACE. Nothing in the interface calls `workspace_open`: choosing a
	@#    directory is a screen a later feature owns, so without this the app shows
	@#    "No workspace open" and nothing else, forever.
	@# 3. Its own profile, so an experiment never touches a real one.
	@#
	@# All three are debug-only. A released build ignores the last two.
	@test -d "$(WS)" || { echo "WS=$(WS) is not a directory"; exit 1; }
	@test -d "$(WS)/.git" || echo "note: $(WS) is not a git repository -- the tree works, git shows nothing (FR-027)"
	npm run ds:sync
	npm run build
	cargo build --workspace
	@mkdir -p "$(LOCAL_PROFILE)"
	@echo "engine:    $(CURDIR)/target/debug/ide-engine"
	@echo "workspace: $(WS)"
	@echo "profile:   $(CURDIR)/$(LOCAL_PROFILE)   (log: $(LOCAL_PROFILE)/shell.log)"
	APEX_LOCAL_ENGINE=$(CURDIR)/target/debug/ide-engine \
	APEX_OPEN_WORKSPACE=$(WS) \
	APEX_DATA_DIR=$(CURDIR)/$(LOCAL_PROFILE) \
	$(XVFB) ./target/debug/apex-shell

local-shot: ## Photograph a local run with a workspace open — works headless (WS=/path/to/repo)
	@# The answer for this box, which has no display. `make local` runs the app where nobody
	@# can see it; this runs the same thing and writes a PNG. The capture harness overrides
	@# APEX_DATA_DIR with its own profile, so this leaves $(LOCAL_PROFILE) alone.
	@test -d "$(WS)" || { echo "WS=$(WS) is not a directory"; exit 1; }
	npm run ds:sync
	npm run build
	cargo build --workspace
	APEX_LOCAL_ENGINE=$(CURDIR)/target/debug/ide-engine \
	APEX_OPEN_WORKSPACE=$(WS) \
	$(XVFB) node scripts/screenshot.mjs $(SHOT)

local-reset: ## Throw away the local run's session and cache
	rm -rf "$(LOCAL_PROFILE)"

shot: ## Photograph the running app to $(SHOT) — works headless
	npm run ds:sync
	npm run build
	cargo build --workspace
	xvfb-run -a -s "-screen 0 1400x900x24" node scripts/screenshot.mjs $(SHOT)

test: ## The full gate: rust, frontend, lint, format
	# --examples builds the fixture programs the task tests spawn. Without it a stale or
	# absent fixture fails those tests for a reason that has nothing to do with terminals.
	cargo build -p apex-engine --bins --examples
	cargo test --workspace
	cargo clippy --workspace --all-targets -- -D warnings
	cargo fmt --all --check
	npm run test:unit
	npm run lint
	npm run lint:ds
	python3 scripts/pipeline_test.py

# A display the windowed steps can actually map a window on. `shot` already did this; `gate`
# defaulted DISPLAY to `:77` and nothing ever started a server there, so a headless run died at
# session creation with "Request timed out" three minutes in -- a message that names neither the
# display nor the cause, and reads as thirty-two broken specs rather than one absent dependency.
#
# That default was worse than no default. WebdriverIO ships `@wdio/xvfb`, which starts a server
# itself when none is present -- and it skips when DISPLAY is set, so `:77` suppressed the very
# fallback that would have rescued the run.
#
# Wrapping here rather than deleting the variable and leaving it to `@wdio/xvfb`, for two
# reasons: `gate:fidelity` is not WebdriverIO and needs a display of its own, so the alternative
# is two mechanisms instead of one; and a fixed 1400x900 makes the run reproducible, which a
# suite comparing rendered geometry against the prototype depends on.
#
# Empty when a real display exists, so a desktop run is unchanged.
XVFB = $(if $(DISPLAY),,xvfb-run -a -s "-screen 0 1400x900x24")

gate: test ## Everything in `test`, plus the end-to-end suite and the fidelity gate
	$(XVFB) npm run e2e
	# A second run, with a real engine in scope. Separate because an engine changes which
	# adapter the status bar observes, which the stub-driven specs in the first run depend on.
	$(XVFB) npm run e2e:live
	$(XVFB) npm run gate:fidelity
	$(MAKE) no-network

no-network: ## Prove the suite needs no network (FR-028, A-TEST, SC-013)
	@# Under an unprivileged user namespace with no network interfaces, a test that reaches
	@# for a socket fails rather than quietly succeeding against something real. Where
	@# unprivileged namespaces are unavailable the check degrades to asserting that no test
	@# names a routable address, which is weaker and says so rather than passing silently.
	@if unshare -rn true 2>/dev/null; then \
		echo "no-network: running the Rust suite with no interfaces"; \
		unshare -rn cargo test --workspace --quiet; \
	else \
		echo "no-network: unprivileged user namespaces unavailable; falling back to a weaker check"; \
		if grep -rn --include=*.rs -E '(127\.0\.0\.1|0\.0\.0\.0|https?://)' engine/src client/core/src | grep -v '^.*://example' ; then \
			echo "no-network: a source file names a network address"; exit 1; \
		fi; \
		echo "no-network: no source file names a network address"; \
	fi

clean: ## Remove build output
	cargo clean
	rm -rf dist reports/screenshots reports/fidelity

next: ## Where the next feature stands and what to run (F=F005 for a specific one)
	@python3 scripts/pipeline.py next $(F)

verify: ## Checks for a feature's current phase; PHASE=propagation for the amendment check
	@python3 scripts/pipeline.py verify $(F) $(if $(PHASE),--phase $(PHASE),)

pipeline: ## Drive phases to the next human gate; add EXECUTE=1 to actually invoke claude
	@python3 scripts/pipeline.py run $(F) $(if $(EXECUTE),--execute,)
