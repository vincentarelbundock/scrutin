.PHONY: help install demo tinytable watch depmap web sync-webview vscode positron rstudio editors docs docs-serve revert stage-docs release release-draft version bump

.DEFAULT_GOAL := help

help: ## Display this help screen
	@echo -e "\033[1mAvailable commands:\033[0m\n"
	@grep -E '^[a-z.A-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}' | sort

# ==============================================================================
# Build / install
# ==============================================================================

# Workspace version, parsed from the first `version = "..."` line in Cargo.toml.
VERSION := $(shell awk -F'"' '/^version/ { print $$2; exit }' Cargo.toml)

install: ## Install the scrutin binary via cargo install
	cargo install --path crates/scrutin-bin

version: ## Print the current workspace version
	@echo $(VERSION)

# Bump the workspace to a new version in all three places the string lives:
# `workspace.package.version`, the three `workspace.dependencies` entries for
# internal crates, and `Cargo.lock`. Usage: `make bump VERSION=0.0.2`.
bump: ## Bump workspace version (usage: make bump VERSION=x.y.z)
	@if [ -z "$(VERSION)" ] || [ "$(VERSION)" = "$(shell awk -F'"' '/^version/ { print $$2; exit }' Cargo.toml)" ]; then \
	    echo "usage: make bump VERSION=x.y.z  (must differ from current $(shell awk -F'"' '/^version/ { print $$2; exit }' Cargo.toml))"; \
	    exit 1; \
	fi
	@sed -i.bak -E 's/^version = "[^"]*"/version = "$(VERSION)"/' Cargo.toml && rm Cargo.toml.bak
	@sed -i.bak -E 's#(path = "crates/scrutin-(core|tui|web)",[[:space:]]*version = )"[^"]*"#\1"$(VERSION)"#g' Cargo.toml && rm Cargo.toml.bak
	@cargo update -w >/dev/null
	@echo "Bumped workspace to $(VERSION)."
	@git diff --stat Cargo.toml Cargo.lock
	@echo ""
	@echo "Next: update CHANGELOG.md, commit Cargo.toml + Cargo.lock, then 'make release'."

# Create a published GitHub Release for the current workspace version. This
# triggers .github/workflows/release.yml, which builds binaries for every
# target and publishes the crates to crates.io. Refuses to run on a dirty
# tree so the tag actually reflects what's on disk.
release: ## Create a published GitHub Release for the current version (fires CI)
	@test -z "$$(git status --porcelain)" || { echo "working tree is dirty; commit or stash first"; exit 1; }
	@echo "Creating release v$(VERSION) from $$(git rev-parse --short HEAD)"
	gh release create v$(VERSION) --title "v$(VERSION)" --generate-notes

# Same as `release` but leaves the release as a draft so you can review it
# before publishing (publishing is what fires the workflow).
release-draft: ## Create a draft GitHub Release (publish from the UI to fire CI)
	@test -z "$$(git status --porcelain)" || { echo "working tree is dirty; commit or stash first"; exit 1; }
	gh release create v$(VERSION) --title "v$(VERSION)" --generate-notes --draft

# ==============================================================================
# Run / dev
# ==============================================================================

demo: ## Run scrutin against the demo/ fixture
	uv run cargo run -- demo/

tinytable: ## Run scrutin against ~/repos/tinytable
	cargo run -- ~/repos/tinytable

watch: ## Run scrutin against ~/repos/tinytable in watch mode
	cargo run -- ~/repos/tinytable --watch

depmap: ## Build the dep map for ~/repos/tinytable
	cargo run -- ~/repos/tinytable --build-depmap

web: ## Run the web dashboard against demo/
	cargo run -- --web demo

revert: ## Restore demo lint files (demo/R/lint.R, demo/src/.../lint.py) to unfixed state
	git checkout -- demo/R/lint.R demo/src/scrutindemo_py/lint.py

# ==============================================================================
# Editor extensions
# ==============================================================================

WEB_FRONTEND := crates/scrutin-web/frontend
VS_WEBVIEW   := editors/vscode/webview

sync-webview: ## Sync the web frontend into the VS Code webview bundle
	@# app.js: straight copy
	cp $(WEB_FRONTEND)/app.js $(VS_WEBVIEW)/app.js
	@# style.css: VS Code variable overrides + web rules (skip :root blocks)
	cat editors/vscode/webview-overrides.css > $(VS_WEBVIEW)/style.css.tmp
	tail -n +41 $(WEB_FRONTEND)/style.css >> $(VS_WEBVIEW)/style.css.tmp
	mv $(VS_WEBVIEW)/style.css.tmp $(VS_WEBVIEW)/style.css
	@# index.html: web version with VS Code template vars
	sed \
		-e 's|<title>scrutin</title>|<meta http-equiv="Content-Security-Policy" content="{{csp}}" />\n  <title>Scrutin</title>|' \
		-e 's|<link rel="stylesheet" href="/style.css" />|<link rel="stylesheet" href="{{styleUri}}" />|' \
		-e 's|<script src="/app.js"></script>|<script nonce="{{nonce}}">window.__SCRUTIN_BASE_URL__ = "{{baseUrl}}";</script>\n  <script nonce="{{nonce}}" src="{{scriptUri}}"></script>|' \
		-e '/<button id="btn-theme"/d' \
		$(WEB_FRONTEND)/index.html > $(VS_WEBVIEW)/index.html

vscode: install sync-webview ## Build and install the VS Code extension
	rm -f editors/vscode/scrutin-*.vsix
	cd editors/vscode && npx tsc -p ./ && npx vsce package --no-dependencies
	code --uninstall-extension scrutin.scrutin 2>/dev/null || true
	code --install-extension editors/vscode/scrutin-0.0.1.vsix --force
	@echo ">>> Reload VS Code window to pick up the new extension <<<"

POSITRON_CLI := /Applications/Positron.app/Contents/Resources/app/bin/code

positron: install sync-webview ## Build and install the Positron extension
	rm -f editors/vscode/scrutin-*.vsix
	cd editors/vscode && npx tsc -p ./ && npx vsce package --no-dependencies
	$(POSITRON_CLI) --uninstall-extension scrutin.scrutin 2>/dev/null || true
	$(POSITRON_CLI) --install-extension editors/vscode/scrutin-0.0.1.vsix --force

rstudio: ## Install the RStudio addin
	R CMD INSTALL editors/rstudio

editors: vscode positron rstudio ## Install all editor extensions (VS Code + Positron + RStudio)

# ==============================================================================
# Documentation
#
# docs-src/ is the lean source tree. Before zensical runs we:
#   1. Regenerate the CLI + config template via `scrutin generate-docs`.
#   2. Materialize a staged copy of docs-src at target/staged-docs-src/,
#      then inject the rendered TOML between the <!-- BEGIN/END
#      init_template.toml --> markers in the staged configuration.md.
#   3. Rewrite zensical.toml into target/staged-zensical.toml pointing
#      at the staged tree, and run zensical against that.
# The working tree's docs-src/ is never mutated by the build.
# ==============================================================================

STAGED_DOCS     := target/staged-docs-src
# Placed at the repo root (not under target/) because zensical resolves
# docs_dir and site_dir relative to the config file and rejects "..".
STAGED_ZENSICAL := .zensical-staged.toml
CONFIG_TEMPLATE := target/docs/configuration-template.toml
CONFIG_MD       := $(STAGED_DOCS)/reference/configuration.md
CLI_MD          := $(STAGED_DOCS)/reference/cli.md
HISTORY_MD      := $(STAGED_DOCS)/history.md
SQL_DIR         := crates/scrutin-core/src/storage/sql

stage-docs: ## Stage docs-src/ with injected CLI/config/SQL into target/
	cargo run --features generate-docs -- generate-docs target/docs
	rm -rf $(STAGED_DOCS)
	cp -R docs-src $(STAGED_DOCS)
	cp target/docs/cli-reference.md $(CLI_MD)
	sed 's/^# Command-Line Help for .*/# Command-Line/; s/—/: /g; s/–/-/g; s/:  /: /g' $(CLI_MD) > $(CLI_MD).tmp
	mv $(CLI_MD).tmp $(CLI_MD)
	@awk 'BEGIN{s=0} \
	/^<!-- BEGIN init_template.toml -->/{print; print "```toml"; while((getline l < "$(CONFIG_TEMPLATE)") > 0) print l; print "```"; s=1; next} \
	/^<!-- END init_template.toml -->/{s=0} \
	!s' $(CONFIG_MD) > $(CONFIG_MD).tmp
	@mv $(CONFIG_MD).tmp $(CONFIG_MD)
	@for f in $(SQL_DIR)/*.sql; do \
	    name=$$(basename $$f); \
	    awk -v path="$$f" -v name="$$name" ' \
	        BEGIN { beginre = "^<!-- BEGIN " name " -->$$"; endre = "^<!-- END " name " -->$$" } \
	        $$0 ~ beginre { print; print "```sql"; while((getline l < path) > 0) print l; close(path); print "```"; skip=1; next } \
	        $$0 ~ endre { skip=0 } \
	        !skip { print }' $(HISTORY_MD) > $(HISTORY_MD).tmp && mv $(HISTORY_MD).tmp $(HISTORY_MD); \
	done
	sed 's|docs_dir = "docs-src"|docs_dir = "$(STAGED_DOCS)"|' zensical.toml > $(STAGED_ZENSICAL)

docs: stage-docs ## Build the documentation site
	uv run zensical build -f $(STAGED_ZENSICAL)

docs-serve: stage-docs ## Serve the documentation site locally
	uv run zensical serve -f $(STAGED_ZENSICAL)
	uv run zensical serve
