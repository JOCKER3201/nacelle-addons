# Installer for the nacelle-desktop widgets.
#
#   make install       — install for the current user (~/.local)
#   sudo make install  — install system-wide (/usr/local)
#   make uninstall     — remove what old versions of this repository
#                        installed as compiled plugins
#
# A widget is one file in a directory named after it: a Rhai script
# under ./widgets/<category>/. The category directory says which kind
# of board the widget may be placed on: board/ for the ordinary boards,
# appgrid/ for the APPGRID (bottom) fixture, search_and_ai/ for the
# SEARCH AND AI (top) one. nacelle-desktop reads whatever it finds
# installed, so this Makefile is how widgets reach the program.
#
# The plugin crates under ./plugins are NOT installed from here any
# more: the control panel, the terminal, the file browser and the
# keyboard are linked into nacelle-desktop itself, so the program works
# with nothing installed and no file can take them away. The crates
# remain here because this is where their code lives — the desktop
# pulls them as dependencies.
#
# The prefix can be overridden: make install PREFIX=/opt/nacelle-desktop

ifeq ($(shell id -u),0)
PREFIX ?= /usr/local
else
PREFIX ?= $(HOME)/.local
endif

WIDGETDIR = $(DESTDIR)$(PREFIX)/share/nacelle-desktop/widgets

.PHONY: all install uninstall clean

all:
	@echo "nothing to build — scripts install as they are; run make install"

install:
	@# Scripts are installed only where they are MISSING, so a widget the
	@# user has edited survives an upgrade untouched.
	@for c in widgets/*/; do \
		[ -d "$$c" ] || continue; \
		cat=$$(basename "$$c"); \
		for d in "$$c"*/; do \
			[ -d "$$d" ] || continue; \
			name=$$(basename "$$d"); \
			mkdir -p "$(WIDGETDIR)/$$cat/$$name"; \
			n=0; kept=0; \
			for f in "$$d"*; do \
				[ -f "$$f" ] || continue; \
				dest="$(WIDGETDIR)/$$cat/$$name/$$(basename "$$f")"; \
				if [ -e "$$dest" ]; then kept=$$((kept+1)); continue; fi; \
				install -Dm644 "$$f" "$$dest"; \
				n=$$((n+1)); \
			done; \
			echo "widget $$cat/$$name: $$n installed, $$kept kept"; \
		done; \
	done; true
	@# Scripts installed by versions from before the category split sit
	@# directly under the widgets directory. The program still reads
	@# them (as board widgets), but next to the copy under board/ they
	@# would be one widget twice — so the old flat copies of OUR
	@# scripts are removed. Only files this repository ships are
	@# touched; a user's own flat widgets stay where they are.
	@for d in widgets/board/*/; do \
		name=$$(basename "$$d"); \
		old="$(WIDGETDIR)/$$name"; \
		[ -d "$$old" ] || continue; \
		rm -f "$$old/$$name.rhai"; \
		rmdir "$$old" 2>/dev/null || true; \
		echo "removed pre-split copy of $$name"; \
	done; true
	@echo "widgets installed in $(WIDGETDIR)"

uninstall:
	@# Old versions installed the four core widgets as compiled .so
	@# plugins; they are part of the program now, so any leftovers are
	@# only dead weight the loader has to skip.
	@for name in control filesystem keyboard shell; do \
		rm -f "$(WIDGETDIR)/$$name/$$name.so" "$(WIDGETDIR)/board/$$name/$$name.so"; \
		rmdir "$(WIDGETDIR)/$$name" "$(WIDGETDIR)/board/$$name" 2>/dev/null || true; \
	done; true
	@echo "compiled plugin leftovers removed from $(WIDGETDIR); scripts were kept"

clean:
	rm -rf target plugins/*/target
