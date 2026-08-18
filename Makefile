PREFIX ?= /usr
PKGNAME = asus-power-manager

all:
	@echo "Run 'make install' as root to install, or './build-packages.sh' to build .deb and .rpm packages."

install:
	install -d $(DESTDIR)$(PREFIX)/bin
	install -d $(DESTDIR)$(PREFIX)/libexec/$(PKGNAME)/scripts
	install -d $(DESTDIR)$(PREFIX)/share/applications
	install -d $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps
	install -d $(DESTDIR)$(PREFIX)/share/$(PKGNAME)/icons/lucide
	install -d $(DESTDIR)/etc/systemd/logind.conf.d
	install -d $(DESTDIR)/etc/sudoers.d

	install -m 0755 src/asus-power-manager $(DESTDIR)$(PREFIX)/bin/asus-power-manager
	install -m 0755 scripts/* $(DESTDIR)$(PREFIX)/libexec/$(PKGNAME)/scripts/
	install -m 0644 data/com.rezkycodes.BatteryManager.desktop $(DESTDIR)$(PREFIX)/share/applications/
	install -m 0644 data/icons/hicolor/scalable/apps/com.rezkycodes.BatteryManager.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/
	install -m 0644 data/icons/lucide/*.svg $(DESTDIR)$(PREFIX)/share/$(PKGNAME)/icons/lucide/
	install -m 0644 data/systemd/clamshell-server.conf $(DESTDIR)/etc/systemd/logind.conf.d/
	install -m 0440 data/sudoers/asus-power-manager $(DESTDIR)/etc/sudoers.d/asus-power-manager

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/asus-power-manager
	rm -rf $(DESTDIR)$(PREFIX)/libexec/$(PKGNAME)
	rm -rf $(DESTDIR)$(PREFIX)/share/$(PKGNAME)
	rm -f $(DESTDIR)$(PREFIX)/share/applications/com.rezkycodes.BatteryManager.desktop
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/com.rezkycodes.BatteryManager.svg
	rm -f $(DESTDIR)/etc/systemd/logind.conf.d/clamshell-server.conf
	rm -f $(DESTDIR)/etc/sudoers.d/asus-power-manager
