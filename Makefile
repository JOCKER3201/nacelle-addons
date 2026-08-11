# Installer for the nacelle-desktop addons.
#
#   make install       — install for the current user (~/.local)
#   sudo make install  — install system-wide (/usr/local)
#   make uninstall     — remove the shipped scripts (edited files are
#                        kept) and sweep leftovers of older layouts
#
# The addons directory holds exactly two kinds of file, flat:
# scripts/<name>.rhai and plugins/<name>.so — the file IS the addon,
# its stem is its name. The category that used to live in a directory
# name lives in the addon itself now: a script declares it in a header
# pragma (`// category: appgrid`) within its first lines, and an addon
# that names none is a board widget.
#
# The plugin crates under ./plugins are NOT installed from here: the
# control panel, the terminal, the file browser and the keyboard are
# linked into nacelle-desktop itself, so the program works with
# nothing installed and no file can take them away. The crates remain
# here because this is where their code lives — the desktop pulls
# them as dependencies. The plugins/ install directory exists for
# THIRD-PARTY compiled addons.
#
# The prefix can be overridden: make install PREFIX=/opt/nacelle-desktop

ifeq ($(shell id -u),0)
PREFIX ?= /usr/local
else
PREFIX ?= $(HOME)/.local
endif

DATADIR  = $(DESTDIR)$(PREFIX)/share/nacelle-desktop
ADDONDIR = $(DATADIR)/addons

.PHONY: all install uninstall clean

all:
	@echo "nothing to build — scripts install as they are; run make install"

install:
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
	echo "addons: $$n installed, $$kept kept"
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
	@echo "addons installed in $(ADDONDIR)"

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
	@# Older layouts: the per-directory widgets tree, and the four core
	@# widgets once installed as compiled plugins — part of the program
	@# now, so any leftovers are only dead weight the loader skips.
	@rm -rf "$(DATADIR)/widgets"
	@for name in control filesystem keyboard shell; do \
		rm -f "$(ADDONDIR)/plugins/$$name.so"; \
	done; true
	@find "$(ADDONDIR)" -type d -empty -delete 2>/dev/null || true
	@echo "shipped addons removed from $(ADDONDIR); edited files were kept"

clean:
	rm -rf target plugins/*/target
