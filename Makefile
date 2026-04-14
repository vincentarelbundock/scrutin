.PHONY: install demo tinytable watch depmap web sync-webview vscode positron rstudio editors docs docs-serve revert stage-docs

install:
	cargo install --path crates/scrutin-bin

demo:
	uv run cargo run -- demo/

tinytable:
	cargo run -- ~/repos/tinytable

watch:
	cargo run -- ~/repos/tinytable --watch

depmap:
	cargo run -- ~/repos/tinytable --build-depmap

web:
	cargo run -- --web demo

WEB_FRONTEND := crates/scrutin-web/frontend
VS_WEBVIEW   := editors/vscode/webview

sync-webview:
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

vscode: install sync-webview
	rm -f editors/vscode/scrutin-*.vsix
	cd editors/vscode && npx tsc -p ./ && npx vsce package --no-dependencies
	code --uninstall-extension scrutin.scrutin 2>/dev/null || true
	code --install-extension editors/vscode/scrutin-0.0.1.vsix --force
	@echo ">>> Reload VS Code window to pick up the new extension <<<"

POSITRON_CLI := /Applications/Positron.app/Contents/Resources/app/bin/code

positron: install sync-webview
	rm -f editors/vscode/scrutin-*.vsix
	cd editors/vscode && npx tsc -p ./ && npx vsce package --no-dependencies
	$(POSITRON_CLI) --uninstall-extension scrutin.scrutin 2>/dev/null || true
	$(POSITRON_CLI) --install-extension editors/vscode/scrutin-0.0.1.vsix --force

rstudio:
	R CMD INSTALL editors/rstudio

editors: vscode positron rstudio

revert:
	git checkout -- demo/R/lint.R demo/src/scrutindemo_py/lint.py

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

STAGED_DOCS     := target/staged-docs-src
# Placed at the repo root (not under target/) because zensical resolves
# docs_dir and site_dir relative to the config file and rejects "..".
STAGED_ZENSICAL := .zensical-staged.toml
CONFIG_TEMPLATE := target/docs/configuration-template.toml
CONFIG_MD       := $(STAGED_DOCS)/reference/configuration.md
CLI_MD          := $(STAGED_DOCS)/reference/cli.md
HISTORY_MD      := $(STAGED_DOCS)/history.md
SQL_DIR         := crates/scrutin-core/src/storage/sql

stage-docs:
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

docs: stage-docs
	uv run zensical build -f $(STAGED_ZENSICAL)

docs-serve: stage-docs
	uv run zensical serve -f $(STAGED_ZENSICAL)
	uv run zensical serve
