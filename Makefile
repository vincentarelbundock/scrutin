.PHONY: help install demo tinytable watch depmap web sync-webview vscode positron rstudio editors docs docs-serve inject-docs revert release version bump

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

# Tag the current commit and push the tag. That triggers BOTH workflows:
#   - .github/workflows/release.yml  (cargo-dist: binaries, installers,
#                                     creates the GitHub Release)
#   - .github/workflows/publish-crates.yml  (cargo publish to crates.io)
# Refuses to run on a dirty tree so the tag reflects what's on disk.
release: ## Tag and push v$(VERSION); fires cargo-dist + crates.io workflows
	@test -z "$$(git status --porcelain)" || { echo "working tree is dirty; commit or stash first"; exit 1; }
	@echo "Tagging v$(VERSION) at $$(git rev-parse --short HEAD) and pushing..."
	git tag -a v$(VERSION) -m "Release v$(VERSION)"
	git push origin v$(VERSION)

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
# Three pages are generated from inputs outside docs-src/ and are gitignored:
#
#   docs-src/reference/cli.md           <- target/docs/cli-reference.md
#                                          (from `cargo run -- generate-docs`)
#   docs-src/reference/configuration.md <- docs-src/reference/configuration.md.in
#                                          + target/docs/configuration-template.toml
#   docs-src/history.md                 <- docs-src/history.md.in
#                                          + crates/scrutin-core/src/storage/sql/*.sql
#
# `make inject-docs` regenerates all three in place. Both `docs` and
# `docs-serve` depend on it. `docs-serve` then runs `zensical serve` directly
# on docs-src/ so live-reload picks up every edit instantly. Edits to the
# *.md.in templates or cargo-produced inputs require re-running
# `make inject-docs` to refresh the generated pages.
# ==============================================================================

CONFIG_TEMPLATE := target/docs/configuration-template.toml
CLI_SRC         := target/docs/cli-reference.md
CONFIG_MD       := docs-src/reference/configuration.md
CONFIG_IN       := $(CONFIG_MD).in
CLI_MD          := docs-src/reference/cli.md
HISTORY_MD      := docs-src/history.md
HISTORY_IN      := $(HISTORY_MD).in
SQL_DIR         := crates/scrutin-core/src/storage/sql
PALETTE_SRC     := crates/scrutin-web/frontend/catppuccin-palette.css
PALETTE_DOCS    := docs-src/stylesheets/catppuccin-palette.css

inject-docs: ## Regenerate injected docs pages (CLI / config / SQL schema) in docs-src/
	cargo run --features generate-docs -- generate-docs target/docs
	@# Shared palette: single source of truth is the dashboard copy.
	@# Keep the docs copy in sync so both sites stay visually aligned.
	@cp $(PALETTE_SRC) $(PALETTE_DOCS)
	sed 's/^# Command-Line Help for .*/# Command-Line/; s/—/: /g; s/–/-/g; s/:  /: /g' $(CLI_SRC) > $(CLI_MD)
	@awk 'BEGIN{s=0} \
	/^<!-- BEGIN init_template.toml -->/{print; print "```toml"; while((getline l < "$(CONFIG_TEMPLATE)") > 0) print l; print "```"; s=1; next} \
	/^<!-- END init_template.toml -->/{s=0} \
	!s' $(CONFIG_IN) > $(CONFIG_MD)
	@cp $(HISTORY_IN) $(HISTORY_MD)
	@for f in $(SQL_DIR)/*.sql; do \
	    name=$$(basename $$f); \
	    awk -v path="$$f" -v name="$$name" ' \
	        BEGIN { beginre = "^<!-- BEGIN " name " -->$$"; endre = "^<!-- END " name " -->$$" } \
	        $$0 ~ beginre { print; print "```sql"; while((getline l < path) > 0) print l; close(path); print "```"; skip=1; next } \
	        $$0 ~ endre { skip=0 } \
	        !skip { print }' $(HISTORY_MD) > $(HISTORY_MD).tmp && mv $(HISTORY_MD).tmp $(HISTORY_MD); \
	done

docs: inject-docs ## Build the static documentation site
	uv run zensical build
	@# zensical copies *.md.in templates into the output verbatim; strip them.
	@find docs -name '*.md.in' -delete

docs-serve: inject-docs ## Serve docs-src/ with live-reload on edits
	uv run zensical serve
