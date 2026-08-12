# Installer for the nacelle-desktop addons.
#
#   make               — build the compiled addons (cargo, release)
#   make install       — build, then install for the current user
#                        (~/.local)
#   sudo make install  — build, then install system-wide (/usr/local)
#   make uninstall     — remove the shipped addons (an edited script is
#                        kept) and sweep leftovers of older layouts
#
# `install` builds what it installs, the same contract nacelle-desktop's
# own Makefile has, so one `make install` per repository is still the
# whole story. A packager who has already built can install without a
# rebuild with `make install-scripts install-plugins`.
#
# The addons directory holds exactly two kinds of file, flat:
# scripts/<name>.rhai and plugins/<name>.so — the file IS the addon,
# its stem is its name. Everything the program must know before an
# addon draws lives in the addon itself: a script declares it in header
# pragmas within its first lines (`// label:`, `// ref_h:`, `// min_h:`,
# `// category:`), a compiled plugin in the `<name>.meta` file installed
# beside its library. Whatever an addon does not declare it is simply
# given — its name in capitals, the standard heights, a board widget.
#
# The plugin crates under ./plugins ARE installed from here, exactly
# like the scripts and exactly like anybody else's compiled addon: the
# core is not a privileged kind of widget, it is the widgets this
# repository happens to ship. Cargo names a cdylib after its crate
# (libnacelle_widget_shell.so); the addon's name is its FILE's stem,
# so each is installed as <name>.so with its <name>.meta beside it.
#
# A .so is overwritten on install where a script is not. A script is
# text somebody may have edited and an upgrade must not silently undo
# that; a shared object is a build artefact of this repository, and a
# stale one against a newer toolkit is a plugin the loader turns away.
#
# The prefix can be overridden: make install PREFIX=/opt/nacelle-desktop

ifeq ($(shell id -u),0)
PREFIX ?= /usr/local
else
PREFIX ?= $(HOME)/.local
endif

DATADIR  = $(DESTDIR)$(PREFIX)/share/nacelle-desktop
ADDONDIR = $(DATADIR)/addons

CARGO     ?= cargo
# Honoured so a build with CARGO_TARGET_DIR set is still found here.
TARGETDIR ?= $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)
RELDIR     = $(TARGETDIR)/release

# The compiled addons this repository ships, by ADDON name. The crate
# is nacelle-widget-NAME and cargo names its cdylib after the crate,
# libnacelle_widget_NAME.so — the mapping is mechanical, so adding a
# widget crate means adding its name here and nowhere else.
# launcher-core is deliberately absent: it is the launcher's shared
# half, not a widget, and it has no attach symbol to export.
PLUGINS = ai appcats appgrid control filesystem keyboard search shell

.PHONY: all plugins install install-scripts install-plugins uninstall clean

all: plugins

plugins:
	$(CARGO) build --release

install: install-scripts install-plugins
	@echo "addons installed in $(ADDONDIR)"

# Depends on the build rather than merely following it in `install`'s
# list, so that `make -j install` cannot start copying libraries a
# build is still writing.
install-plugins: plugins
	@# Compiled addons: overwritten, never kept — see the head of this
	@# file. A widget whose .so is not there is REPORTED rather than
	@# passed over in silence: an addon directory missing it is a
	@# program missing that widget, which is a thing to be told.
	@mkdir -p "$(ADDONDIR)/plugins"
	@n=0; missing=""; \
	for name in $(PLUGINS); do \
		so="$(RELDIR)/libnacelle_widget_$$name.so"; \
		if [ ! -f "$$so" ]; then missing="$$missing $$name"; continue; fi; \
		install -Dm755 "$$so" "$(ADDONDIR)/plugins/$$name.so"; \
		if [ -f "plugins/$$name/$$name.meta" ]; then \
			install -Dm644 "plugins/$$name/$$name.meta" \
				"$(ADDONDIR)/plugins/$$name.meta"; \
		fi; \
		n=$$((n+1)); \
	done; \
	echo "plugins: $$n installed"; \
	if [ -n "$$missing" ]; then \
		echo "plugins: MISSING FROM $(RELDIR) —$$missing"; \
		echo "         these widgets will not exist; run 'make' and"; \
		echo "         read what cargo says about them"; \
	fi

install-scripts:
	@# Scripts are installed only where they are MISSING, so an addon
	@# the user has edited survives an upgrade untouched.
	@mkdir -p "$(ADDONDIR)/scripts" "$(ADDONDIR)/plugins"
	@n=0; kept=0; \
	for f in scripts/*.rhai; do \
		[ -f "$$f" ] || continue; \
		dest="$(ADDONDIR)/scripts/$$(basename "$$f")"; \
		if [ -e "$$dest" ]; then kept=$$((kept+1)); continue; fi; \
		install -Dm644 "$$f" "$$dest"; \
		n=$$((n+1)); \
	done; \
	echo "scripts: $$n installed, $$kept kept"
	@# The pre-addons layout kept every script in its own directory
	@# under widgets/<category>/. The program migrates the user's copy
	@# at startup; a tree this Makefile finds here is one the program
	@# never touched (a system prefix, say) — rescue every script into
	@# the new place (never overwriting) before sweeping it away, and
	@# carry the category directory into the pragma the same way.
	@if [ -d "$(DATADIR)/widgets" ]; then \
		for cat in appgrid search_and_ai; do \
			for f in "$(DATADIR)"/widgets/$$cat/*/*.rhai; do \
				[ -f "$$f" ] || continue; \
				dest="$(ADDONDIR)/scripts/$$(basename "$$f")"; \
				[ -e "$$dest" ] && continue; \
				{ printf '// category: %s\n' "$$cat"; cat "$$f"; } > "$$dest"; \
				echo "  rescued $$f -> $$dest"; \
			done; \
		done; \
		for f in "$(DATADIR)"/widgets/board/*/*.rhai "$(DATADIR)"/widgets/*/*.rhai; do \
			[ -f "$$f" ] || continue; \
			dest="$(ADDONDIR)/scripts/$$(basename "$$f")"; \
			[ -e "$$dest" ] && continue; \
			cp "$$f" "$$dest"; \
			echo "  rescued $$f -> $$dest"; \
		done; \
		for f in "$(DATADIR)"/widgets/*/*/*.so "$(DATADIR)"/widgets/*/*.so; do \
			[ -f "$$f" ] || continue; \
			dest="$(ADDONDIR)/plugins/$$(basename "$$f")"; \
			[ -e "$$dest" ] && continue; \
			cp "$$f" "$$dest"; \
			echo "  rescued $$f -> $$dest"; \
		done; \
		rm -rf "$(DATADIR)/widgets"; \
		echo "retired the widgets/ layout"; \
	fi; true

uninstall:
	@# Shipped scripts leave; an edited copy is somebody's work and
	@# stays, exactly like the install's only-where-missing rule.
	@for f in scripts/*.rhai; do \
		[ -f "$$f" ] || continue; \
		dest="$(ADDONDIR)/scripts/$$(basename "$$f")"; \
		[ -e "$$dest" ] || continue; \
		if cmp -s "$$f" "$$dest" 2>/dev/null; then \
			rm -f "$$dest"; \
		else \
			echo "  kept (edited) $$dest"; \
		fi; \
	done; true
	@# Compiled addons leave outright — nobody edits a .so, so there is
	@# no work of anybody's to preserve, and a library left behind
	@# after its toolkit has gone is a plugin the loader turns away.
	@for name in $(PLUGINS); do \
		rm -f "$(ADDONDIR)/plugins/$$name.so" \
		      "$(ADDONDIR)/plugins/$$name.meta"; \
	done; true
	@# The pre-addons layout: every script in its own directory under
	@# widgets/<category>/, which nothing reads any more.
	@rm -rf "$(DATADIR)/widgets"
	@find "$(ADDONDIR)" -type d -empty -delete 2>/dev/null || true
	@echo "shipped addons removed from $(ADDONDIR); edited scripts were kept"

clean:
	rm -rf "$(TARGETDIR)" plugins/*/target
