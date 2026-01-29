PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
DESTDIR ?=

.PHONY: all build install uninstall clean deploy

all: build

build:
	cargo build --release --package sootmix
	cargo build --release --package sootmix-daemon
	cargo build --release --package sootmix-rnnoise-ladspa

install:
	@if [ ! -f target/release/sootmix ] || [ ! -f target/release/sootmix-daemon ]; then \
		echo "Error: Binaries not found. Run 'make' first (without sudo)."; \
		exit 1; \
	fi
	# Install binaries
	install -Dm755 target/release/sootmix $(DESTDIR)$(BINDIR)/sootmix
	install -Dm755 target/release/sootmix-daemon $(DESTDIR)$(BINDIR)/sootmix-daemon

	# Install LADSPA plugin for noise suppression
	install -Dm755 target/release/libsootmix_rnnoise_ladspa.so $(DESTDIR)$(BINDIR)/libsootmix_rnnoise_ladspa.so

	# Install desktop file
	install -Dm644 contrib/sootmix.desktop $(DESTDIR)$(DATADIR)/applications/sootmix.desktop

	# Install icon
	install -Dm644 contrib/icons/sootmix.svg $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/sootmix.svg

	# Install systemd user service (substitute correct binary path)
	sed 's|ExecStart=.*|ExecStart=$(BINDIR)/sootmix-daemon|' contrib/sootmix-daemon.service > /tmp/sootmix-daemon.service.tmp
	install -Dm644 /tmp/sootmix-daemon.service.tmp $(DESTDIR)$(PREFIX)/lib/systemd/user/sootmix-daemon.service
	rm -f /tmp/sootmix-daemon.service.tmp

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/sootmix
	rm -f $(DESTDIR)$(BINDIR)/sootmix-daemon
	rm -f $(DESTDIR)$(BINDIR)/libsootmix_rnnoise_ladspa.so
	rm -f $(DESTDIR)$(DATADIR)/applications/sootmix.desktop
	rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/sootmix.svg
	rm -f $(DESTDIR)$(PREFIX)/lib/systemd/user/sootmix-daemon.service

deploy: build
	sudo install -Dm755 target/release/sootmix $(BINDIR)/sootmix
	sudo install -Dm755 target/release/sootmix-daemon $(BINDIR)/sootmix-daemon
	sudo install -Dm755 target/release/libsootmix_rnnoise_ladspa.so $(BINDIR)/libsootmix_rnnoise_ladspa.so
	@if systemctl --user is-active sootmix-daemon.service >/dev/null 2>&1; then \
		systemctl --user restart sootmix-daemon.service; \
		echo "sootmix-daemon restarted."; \
	else \
		echo "sootmix-daemon service not active, skipping restart."; \
	fi

clean:
	cargo clean
