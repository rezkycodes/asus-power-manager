PREFIX ?= /usr
PKGNAME = asus-power-manager
BIN_NAME = asus-tuf-cpu
APP_ID = com.rezkycodes.AsusTufCpu
RUST_BIN = rust-gui/target/release/$(BIN_NAME)

all:
	@echo "Run 'make build' to compile, 'make install' (as root) to install, or './build-packages.sh' for .deb/.rpm."

build:
	cd rust-gui && cargo build --release

$(RUST_BIN): build

install: $(RUST_BIN)
	install -d $(DESTDIR)$(PREFIX)/bin
	install -d $(DESTDIR)$(PREFIX)/libexec/$(PKGNAME)/scripts
	install -d $(DESTDIR)$(PREFIX)/share/applications
	install -d $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps
	install -d $(DESTDIR)$(PREFIX)/share/$(BIN_NAME)/icons
	install -d $(DESTDIR)$(PREFIX)/lib/systemd/system
	install -d $(DESTDIR)$(PREFIX)/lib/udev/rules.d
	install -d $(DESTDIR)$(PREFIX)/lib/modprobe.d
	install -d $(DESTDIR)/etc/systemd/logind.conf.d
	install -d $(DESTDIR)/etc/sysctl.d
	install -d $(DESTDIR)/etc/sudoers.d

	# Rust binary + backward-compat symlink for the old launcher name
	install -m 0755 $(RUST_BIN) $(DESTDIR)$(PREFIX)/bin/$(BIN_NAME)
	ln -sf $(BIN_NAME) $(DESTDIR)$(PREFIX)/bin/asus-power-manager

	# Backend scripts, desktop entry, brand icon, bundled lucide icons
	install -m 0755 scripts/* $(DESTDIR)$(PREFIX)/libexec/$(PKGNAME)/scripts/
	install -m 0644 data/$(APP_ID).desktop $(DESTDIR)$(PREFIX)/share/applications/
	install -m 0644 tweak-asus-tuf.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg
	install -m 0644 rust-gui/icons/*.svg $(DESTDIR)$(PREFIX)/share/$(BIN_NAME)/icons/

	# System stability assets
	install -m 0644 data/systemd/battery-charge-threshold.service $(DESTDIR)$(PREFIX)/lib/systemd/system/
	install -m 0644 data/systemd/clamshell-server.conf $(DESTDIR)/etc/systemd/logind.conf.d/
	install -m 0644 data/udev/*.rules $(DESTDIR)$(PREFIX)/lib/udev/rules.d/
	install -m 0644 data/sysctl/99-io-stability.conf $(DESTDIR)/etc/sysctl.d/
	install -m 0644 data/modprobe/nvidia-power-stability.conf $(DESTDIR)$(PREFIX)/lib/modprobe.d/
	install -m 0440 data/sudoers/asus-power-manager $(DESTDIR)/etc/sudoers.d/asus-power-manager

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/$(BIN_NAME)
	rm -f $(DESTDIR)$(PREFIX)/bin/asus-power-manager
	rm -rf $(DESTDIR)$(PREFIX)/libexec/$(PKGNAME)
	rm -rf $(DESTDIR)$(PREFIX)/share/$(BIN_NAME)
	rm -f $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg
	rm -f $(DESTDIR)$(PREFIX)/lib/systemd/system/battery-charge-threshold.service
	rm -f $(DESTDIR)/etc/systemd/logind.conf.d/clamshell-server.conf
	rm -f $(DESTDIR)$(PREFIX)/lib/udev/rules.d/98-unbind-nvidia-xhci.rules
	rm -f $(DESTDIR)$(PREFIX)/lib/udev/rules.d/99-battery-charge-threshold.rules
	rm -f $(DESTDIR)$(PREFIX)/lib/udev/rules.d/99-battery-tuned.rules
	rm -f $(DESTDIR)$(PREFIX)/lib/udev/rules.d/99-usb-mouse-no-autosuspend.rules
	rm -f $(DESTDIR)/etc/sysctl.d/99-io-stability.conf
	rm -f $(DESTDIR)$(PREFIX)/lib/modprobe.d/nvidia-power-stability.conf
	rm -f $(DESTDIR)/etc/sudoers.d/asus-power-manager

.PHONY: all build install uninstall
