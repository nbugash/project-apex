# Running and looking at the application.
#
# This project is developed on a headless remote box, so "see the GUI" needs two different
# answers: run it on a machine that has a display, or photograph it on one that does not.

SHELL := /bin/bash
SHOT ?= reports/app.png

.DEFAULT_GOAL := help

.PHONY: help setup dev run shot test gate clean next verify pipeline

help: ## Show this help
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | awk -F':.*?## ' '{printf "  \033[1m%-10s\033[0m %s\n", $$1, $$2}'
	@echo
	@echo "  On a machine with a display:  make dev"
	@echo "  On this headless box:         make shot   (writes $(SHOT))"
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

shot: ## Photograph the running app to $(SHOT) — works headless
	npm run ds:sync
	npm run build
	cargo build --workspace
	xvfb-run -a -s "-screen 0 1400x900x24" node scripts/screenshot.mjs $(SHOT)

test: ## The full gate: rust, frontend, lint, format
	cargo build -p apex-engine
	cargo test --workspace
	cargo clippy --workspace --all-targets -- -D warnings
	cargo fmt --all --check
	npm run test:unit
	npm run lint
	npm run lint:ds
	python3 scripts/pipeline_test.py

gate: test ## Everything in `test`, plus the end-to-end suite and the fidelity gate
	xvfb-run -a npm run e2e
	xvfb-run -a npm run gate:fidelity

clean: ## Remove build output
	cargo clean
	rm -rf dist reports/screenshots reports/fidelity

next: ## Where the next feature stands and what to run (F=F005 for a specific one)
	@python3 scripts/pipeline.py next $(F)

verify: ## Checks for a feature's current phase; PHASE=propagation for the amendment check
	@python3 scripts/pipeline.py verify $(F) $(if $(PHASE),--phase $(PHASE),)

pipeline: ## Drive phases to the next human gate; add EXECUTE=1 to actually invoke claude
	@python3 scripts/pipeline.py run $(F) $(if $(EXECUTE),--execute,)
