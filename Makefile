# Installer for the nacelle-desktop addons.
#
#   make               — build the compiled addons (cargo, release)
#   make install       — build, then install for the current user
#                        (~/.local)
#   sudo make install  — build, then install system-wide (/usr/local)
#   make uninstall     — remove the shipped addons (an edited script is
#                        kept), under both directory names
#   make check-install — install into a throwaway DESTDIR and check
#                        WHERE it landed; needs no build
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
# The prefix can be overridden: make install PREFIX=/opt/nacelle

ifeq ($(shell id -u),0)
PREFIX ?= /usr/local
else
PREFIX ?= $(HOME)/.local
endif

# The directory is named after the nacelle FAMILY, not after one of its
# programs — the same name nacelle-themes installs under and the same
# name the program looks in FIRST. It was share/nacelle-desktop here
# until 2026-08-18, and that single wrong word had a visible cost: a
# machine with nothing but a fresh install of this repository on it got
# nacelle-desktop's "reading data from the folder's old name" warning
# on its very first run, because the only data directory in existence
# was the one this installer had just created under the retired name.
#
# Nothing on disk is moved by the change. Both names are READ — the
# program searches the family name first and the old one directly
# behind it, so an installation from before this goes on working
# exactly as it did — and this file reads both too, wherever reading
# is what it does: install-scripts asks both before deciding a script
# is missing (laying a fresh copy under the new name on top of one the
# user edited under the old would SHADOW the edit, which is the same
# silent undo the only-where-missing rule exists to refuse), and
# uninstall takes back what it wrote under either.
#
# Only WRITES moved, which is what makes the change reversible: it
# deletes nothing and it renames nothing.
DATADIR         = $(DESTDIR)$(PREFIX)/share/nacelle
LEGACY_DATADIR  = $(DESTDIR)$(PREFIX)/share/nacelle-desktop
ADDONDIR        = $(DATADIR)/addons
LEGACY_ADDONDIR = $(LEGACY_DATADIR)/addons

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
PLUGINS = aichat ailoop aiphoto aisort appcats appgrid control filesystem keyboard search shell

.PHONY: all plugins install install-scripts install-plugins uninstall \
        check-install clean

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
	@# A copy of a shipped library left under the folder's old name is
	@# SHADOWED, not loaded: the program takes the first directory on
	@# the search path that holds a given name and the family's comes
	@# first. So nothing has to be moved — but it is worth one line,
	@# because the file is a build artefact of this repository and the
	@# day this repository stops shipping that name it would surface
	@# again. `make uninstall` takes it.
	@old=0; \
	for name in $(PLUGINS); do \
		[ -f "$(LEGACY_ADDONDIR)/plugins/$$name.so" ] && old=$$((old+1)); \
	done; \
	[ "$$old" = 0 ] || echo "plugins: $$old older copies remain in \
$(LEGACY_ADDONDIR)/plugins, shadowed by these; 'make uninstall' removes them"; \
	true

install-scripts:
	@# Scripts are installed only where they are MISSING, so an addon
	@# the user has edited survives an upgrade untouched. MISSING is
	@# asked of both directory names: a script standing under the old
	@# one is a script the program still reads, one place further down
	@# the search path, so writing a factory copy above it would undo
	@# the edit exactly as overwriting would — the same silent undo,
	@# committed with a different instrument.
	@mkdir -p "$(ADDONDIR)/scripts" "$(ADDONDIR)/plugins"
	@n=0; kept=0; old=0; \
	for f in scripts/*.rhai; do \
		[ -f "$$f" ] || continue; \
		base="$$(basename "$$f")"; \
		if [ -e "$(ADDONDIR)/scripts/$$base" ]; then kept=$$((kept+1)); continue; fi; \
		if [ -e "$(LEGACY_ADDONDIR)/scripts/$$base" ]; then \
			kept=$$((kept+1)); old=$$((old+1)); continue; \
		fi; \
		install -Dm644 "$$f" "$(ADDONDIR)/scripts/$$base"; \
		n=$$((n+1)); \
	done; \
	echo "scripts: $$n installed, $$kept kept"; \
	[ "$$old" = 0 ] || echo "         $$old of those kept are under the folder's \
old name, in $(LEGACY_ADDONDIR)/scripts — still read, still yours"
	@# The pre-addons layout kept every script in its own directory
	@# under widgets/<category>/. BOTH roots are walked: since the
	@# rename the program migrates only the directory it WRITES to
	@# (share/nacelle), so a widgets/ tree under share/nacelle-desktop
	@# — which is where every installation older than the rename has
	@# one — is a tree nothing else will ever carry forward.
	@#
	@# Nothing is overwritten and nothing beyond the rescued file
	@# itself is deleted: directories go only when they are empty, so
	@# notes or assets somebody left beside a widget outlive it. That
	@# is the same restraint the program's own migration keeps, and it
	@# is what earns this the right to sweep at all.
	@#
	@# `taken` asks BOTH names, and it has to: a machine can hold two
	@# older copies of one addon — an edited one under the old name in
	@# the addons layout and an untouched one in the pre-addons tree —
	@# and rescuing the second over the first would put the FACTORY
	@# text where the program looks first. That is the edit undone, by
	@# the one code path here that was still allowed to do it.
	@taken() { [ -e "$(ADDONDIR)/$$1" ] || [ -e "$(LEGACY_ADDONDIR)/$$1" ]; }; \
	for root in "$(DATADIR)" "$(LEGACY_DATADIR)"; do \
		[ -d "$$root/widgets" ] || continue; \
		for cat in appgrid search_and_ai; do \
			for f in "$$root"/widgets/$$cat/*/*.rhai; do \
				[ -f "$$f" ] || continue; \
				sub="scripts/$$(basename "$$f")"; \
				if taken "$$sub"; then \
					echo "  kept in place (that name is taken) $$f"; \
					continue; \
				fi; \
				{ printf '// category: %s\n' "$$cat"; cat "$$f"; } \
					> "$(ADDONDIR)/$$sub" \
					&& rm -f "$$f" \
					&& echo "  rescued $$f -> $(ADDONDIR)/$$sub"; \
			done; \
		done; \
		for f in "$$root"/widgets/board/*/*.rhai "$$root"/widgets/*/*.rhai; do \
			[ -f "$$f" ] || continue; \
			sub="scripts/$$(basename "$$f")"; \
			if taken "$$sub"; then \
				echo "  kept in place (that name is taken) $$f"; \
				continue; \
			fi; \
			cp "$$f" "$(ADDONDIR)/$$sub" && rm -f "$$f" \
				&& echo "  rescued $$f -> $(ADDONDIR)/$$sub"; \
		done; \
		for f in "$$root"/widgets/*/*/*.so "$$root"/widgets/*/*.so; do \
			[ -f "$$f" ] || continue; \
			sub="plugins/$$(basename "$$f")"; \
			if taken "$$sub"; then \
				echo "  kept in place (that name is taken) $$f"; \
				continue; \
			fi; \
			cp "$$f" "$(ADDONDIR)/$$sub" && rm -f "$$f" \
				&& echo "  rescued $$f -> $(ADDONDIR)/$$sub"; \
		done; \
		find "$$root/widgets" -depth -type d -exec rmdir {} + 2>/dev/null || true; \
		[ -d "$$root/widgets" ] || echo "retired the widgets/ layout in $$root"; \
	done; true

uninstall:
	@# Both names, because this installer wrote under the old one until
	@# 2026-08-18 and those files are its own to take back. Only its
	@# own: share/nacelle belongs to the whole family — nacelle-themes
	@# installs there too — so nothing outside addons/ is touched, and
	@# inside it only the names this repository ships.
	@#
	@# Shipped scripts leave; an edited copy is somebody's work and
	@# stays, exactly like the install's only-where-missing rule. A
	@# compiled addon leaves outright: nobody edits a .so, and a
	@# library left behind after its toolkit has gone is a plugin the
	@# loader turns away.
	@for dir in "$(ADDONDIR)" "$(LEGACY_ADDONDIR)"; do \
		[ -d "$$dir" ] || continue; \
		for f in scripts/*.rhai; do \
			[ -f "$$f" ] || continue; \
			dest="$$dir/scripts/$$(basename "$$f")"; \
			[ -e "$$dest" ] || continue; \
			if cmp -s "$$f" "$$dest" 2>/dev/null; then \
				rm -f "$$dest"; \
			else \
				echo "  kept (edited) $$dest"; \
			fi; \
		done; \
		for name in $(PLUGINS); do \
			rm -f "$$dir/plugins/$$name.so" "$$dir/plugins/$$name.meta"; \
		done; \
		find "$$dir" -type d -empty -delete 2>/dev/null || true; \
	done; true
	@# The pre-addons widgets/ layout is NOT swept here, and that is the
	@# fix rather than the omission: this target ran `rm -rf` over the
	@# whole tree until 2026-08-18, and that tree held THIRD-PARTY
	@# addons beside the shipped ones — uninstalling ours deleted
	@# theirs. Nothing in it was ever put there by this Makefile, which
	@# has only ever installed under addons/. What carries it forward
	@# is `make install`, file by file, keeping what it cannot claim.
	@echo "shipped addons removed from $(ADDONDIR)"
	@echo "  and from $(LEGACY_ADDONDIR); edited scripts were kept"

# A dry run of the install into a throwaway DESTDIR, which answers the
# one question this file had answered wrongly for months: WHERE the
# addons land. It needs no build — install-scripts does not depend on
# one — so it costs a second, and it FAILS rather than printing a tree
# for somebody to read and approve.
#
# Fail-closed on the count as well as on the place: zero scripts
# installed into the right directory is a pass by every check that only
# looks for wrong ones.
#
# MAKEFLAGS is cleared for the recursion because make runs a line
# holding $(MAKE) even under `make -n`, and would then hand -n down to
# the very install this target has to observe: a dry run that installs
# nothing and a check that finds nothing look exactly alike. The whole
# of what the recursion writes is inside a directory the trap deletes.
check-install:
	@tmp="$$(mktemp -d)" || exit 1; \
	trap 'rm -rf "$$tmp"' EXIT; \
	MAKEFLAGS= $(MAKE) --no-print-directory install-scripts \
		DESTDIR="$$tmp" PREFIX=/usr >/dev/null; \
	want="$$tmp/usr/share/nacelle/addons/scripts"; \
	fail=0; \
	for f in scripts/*.rhai; do \
		[ -f "$$f" ] || continue; \
		[ -f "$$want/$$(basename "$$f")" ] || { \
			echo "check-install: NOT INSTALLED $$(basename "$$f")"; fail=1; }; \
	done; \
	if [ -e "$$tmp/usr/share/nacelle-desktop" ]; then \
		echo "check-install: written under the folder's OLD name:"; \
		find "$$tmp/usr/share/nacelle-desktop" | sed 's/^/    /'; \
		fail=1; \
	fi; \
	n=$$(find "$$want" -name '*.rhai' 2>/dev/null | wc -l); \
	[ "$$n" -gt 0 ] || { echo "check-install: nothing was installed at all"; fail=1; }; \
	[ "$$fail" = 0 ] && echo "check-install: $$n scripts in share/nacelle/addons/scripts, \
nothing under share/nacelle-desktop"; \
	exit $$fail

clean:
	rm -rf "$(TARGETDIR)" plugins/*/target
