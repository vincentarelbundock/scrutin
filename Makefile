.PHONY: help install demo tinytable watch depmap web sync-webview vscode positron rstudio editors docs docs-preview inject-docs revert release version bump

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

sync-webview:
	@# esbuild bundles app.js + modules/*; HTML transform strips and injects
	@# CSP/nonce/URI markers. See editors/vscode/build-webview.mjs.
	cd $(VSCODE_DIR) && npm install --no-audit --no-fund --silent
	cd $(VSCODE_DIR) && node build-webview.mjs

vscode: sync-webview vscode-sync-version ## Build and install the VS Code extension with bundled binary
	cargo build --release -p scrutin
	rm -f editors/vscode/scrutin-*.vsix
	rm -rf editors/vscode/bin
	mkdir -p editors/vscode/bin
	cp target/release/scrutin editors/vscode/bin/
	chmod +x editors/vscode/bin/scrutin
	cd editors/vscode && npx tsc -p ./ && npx vsce package --no-dependencies
	code --uninstall-extension VincentArel-Bundock.scrutin-runner 2>/dev/null || true
	code --install-extension editors/vscode/scrutin-runner-$(VERSION).vsix --force
	@echo ">>> Reload VS Code window to pick up the new extension <<<"

UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
  POSITRON_CLI := /Applications/Positron.app/Contents/Resources/app/bin/code
else
  POSITRON_CLI := positron
endif

positron: sync-webview vscode-sync-version ## Build and install the Positron extension with bundled binary
	cargo build --release -p scrutin
	rm -f editors/vscode/scrutin-*.vsix
	rm -rf editors/vscode/bin
	mkdir -p editors/vscode/bin
	cp target/release/scrutin editors/vscode/bin/
	chmod +x editors/vscode/bin/scrutin
	cd editors/vscode && npx tsc -p ./ && npx vsce package --no-dependencies
	$(POSITRON_CLI) --uninstall-extension VincentArel-Bundock.scrutin-runner 2>/dev/null || true
	$(POSITRON_CLI) --install-extension editors/vscode/scrutin-runner-$(VERSION).vsix --force

rstudio: install ## Install the scrutin binary + the RStudio addin (the addin shells out to scrutin on $PATH)
	R CMD INSTALL editors/rstudio

editors: vscode positron rstudio ## Install all editor extensions (VS Code + Positron + RStudio)

# ==============================================================================
# VS Code extension packaging (per-platform, for Marketplace + Open VSX)
# ==============================================================================

VSCODE_DIR := editors/vscode
VSIX_OUT   := $(VSCODE_DIR)/dist

vscode-sync-version:
	@node -e "const fs=require('fs'); \
	  const p='$(VSCODE_DIR)/package.json'; const j=JSON.parse(fs.readFileSync(p)); \
	  j.version='$(VERSION)'; fs.writeFileSync(p, JSON.stringify(j, null, 2)+'\n');"

vscode-stage-binary:
	@test -n "$(BIN_PATH)" || { echo "BIN_PATH must be set"; exit 1; }
	@mkdir -p $(VSCODE_DIR)/bin
	@cp "$(BIN_PATH)" $(VSCODE_DIR)/bin/
	@chmod +x $(VSCODE_DIR)/bin/scrutin* 2>/dev/null || true

vscode-package-target: sync-webview vscode-sync-version
	@test -n "$(TARGET)" || { echo "TARGET must be set (e.g. darwin-arm64)"; exit 1; }
	@mkdir -p $(VSIX_OUT)
	cd $(VSCODE_DIR) && npx tsc -p ./
	cd $(VSCODE_DIR) && npx vsce package --no-dependencies \
	  --target $(TARGET) -o dist/scrutin-$(TARGET)-$(VERSION).vsix

vscode-package-universal: sync-webview vscode-sync-version
	@mkdir -p $(VSIX_OUT)
	@rm -rf $(VSCODE_DIR)/bin
	cd $(VSCODE_DIR) && npx tsc -p ./
	cd $(VSCODE_DIR) && npx vsce package --no-dependencies \
	  -o dist/scrutin-universal-$(VERSION).vsix

vscode-publish:
	@for v in $(VSIX_OUT)/*.vsix; do \
	  echo "Publishing $$v"; \
	  npx --prefix $(VSCODE_DIR) vsce publish --no-dependencies --packagePath $$v -p $$VSCE_PAT; \
	  npx --prefix $(VSCODE_DIR) ovsx publish $$v -p $$OVSX_PAT; \
	done

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
	@# Publish raw markdown sources alongside the rendered HTML so LLMs can
	@# fetch plain markdown from the same origin (e.g. /reporters.md alongside /reporters/).
	@# Preserves subdirectory layout: docs-src/tools/ruff.md → docs/tools/ruff.md.
	rsync -am --include='*/' --include='*.md' --exclude='*' docs-src/ docs/
	@# GitHub Pages runs Jekyll by default, which would re-render the .md
	@# files instead of serving them raw. `.nojekyll` disables Jekyll entirely
	@# so every file (including .md and llms-full.txt) is served as-is.
	@touch docs/.nojekyll
	@# Single-file concatenation of every doc page for agents that want one
	@# fetch. Follows the llmstxt.org convention of shipping llms-full.txt
	@# alongside llms.txt.
	@( echo "# scrutin: full documentation"; echo; \
	   cd docs-src && find . -name '*.md' | LC_ALL=C sort | while read f; do \
	     echo "---"; echo "source: $${f#./}"; echo "---"; echo; \
	     cat "$$f"; echo; \
	   done ) > docs/llms-full.txt

docs-preview: docs ## Build docs and serve statically on :8001 (matches GitHub Pages layout; includes llms.txt + raw .md)
	@echo "Serving $(PWD)/docs on http://127.0.0.1:8001"
	@python3 -m http.server --directory docs 8001
