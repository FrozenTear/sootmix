PREFIX ?= /usr
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
DESTDIR ?=

.PHONY: all build install uninstall clean

all: build

build:
	cargo build --release --package sootmix
	cargo build --release --package sootmix-daemon

install: build
	# Install binaries
	install -Dm755 target/release/sootmix $(DESTDIR)$(BINDIR)/sootmix
	install -Dm755 target/release/sootmix-daemon $(DESTDIR)$(BINDIR)/sootmix-daemon

	# Install desktop file
	install -Dm644 contrib/sootmix.desktop $(DESTDIR)$(DATADIR)/applications/sootmix.desktop

	# Install icon
	install -Dm644 contrib/icons/sootmix.svg $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/sootmix.svg

	# Install systemd user service
	install -Dm644 contrib/sootmix-daemon.service $(DESTDIR)$(PREFIX)/lib/systemd/user/sootmix-daemon.service

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/sootmix
	rm -f $(DESTDIR)$(BINDIR)/sootmix-daemon
	rm -f $(DESTDIR)$(DATADIR)/applications/sootmix.desktop
	rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/sootmix.svg
	rm -f $(DESTDIR)$(PREFIX)/lib/systemd/user/sootmix-daemon.service

clean:
	cargo clean
