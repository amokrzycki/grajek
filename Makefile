PREFIX ?= $(HOME)/.local
DATADIR ?= $(PREFIX)/share
BINARY ?= target/release/grajek

.PHONY: install
install:
	install -Dm755 "$(BINARY)" "$(DESTDIR)$(PREFIX)/bin/grajek"
	install -Dm644 assets/linux/org.grajek.desktop.desktop "$(DESTDIR)$(DATADIR)/applications/org.grajek.desktop.desktop"
	install -Dm644 assets/brand/grajek-mark-dark.svg "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/org.grajek.desktop.svg"
	@set -e; for size in 16 24 32 48 64 128 256 512 1024; do \
		install -Dm644 "assets/brand/grajek-app-$$size.png" "$(DESTDIR)$(DATADIR)/icons/hicolor/$${size}x$${size}/apps/org.grajek.desktop.png"; \
	done
