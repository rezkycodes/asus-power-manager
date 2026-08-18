Name:           asus-power-manager
Version:        1.0.0
Release:        1%{?dist}
Summary:        Power & Battery Manager for ASUS and Linux Laptops

License:        MIT
URL:            https://github.com/rezkycodes/asus-power-manager
BuildArch:      noarch

Requires:       python3
Requires:       python3-gobject
Requires:       libadwaita
Requires:       gtk4
Requires:       upower
Requires:       systemd
Requires:       udev

Recommends:     tuned
Recommends:     hdparm

%description
Modern GTK4/Libadwaita power management and battery health control utility.
Supports battery charge limits (80% health care), instant powersave/performance
switching, clamshell server mode, and hardware stability hardening.

%install
rm -rf %{buildroot}
mkdir -p %{buildroot}%{_bindir}
mkdir -p %{buildroot}%{_libexecdir}/%{name}/scripts
mkdir -p %{buildroot}%{_datadir}/applications
mkdir -p %{buildroot}%{_datadir}/icons/hicolor/scalable/apps
mkdir -p %{buildroot}%{_datadir}/%{name}/icons/lucide
mkdir -p %{buildroot}%{_unitdir}
mkdir -p %{buildroot}%{_udevrulesdir}
mkdir -p %{buildroot}%{_sysctldir}
mkdir -p %{buildroot}%{_prefix}/lib/modprobe.d
mkdir -p %{buildroot}%{_sysconfdir}/systemd/logind.conf.d
mkdir -p %{buildroot}%{_sysconfdir}/sudoers.d

install -m 0755 %{_sourcedir}/src/asus-power-manager %{buildroot}%{_bindir}/asus-power-manager
install -m 0755 %{_sourcedir}/scripts/* %{buildroot}%{_libexecdir}/%{name}/scripts/
install -m 0644 %{_sourcedir}/data/com.rezkycodes.BatteryManager.desktop %{buildroot}%{_datadir}/applications/
install -m 0644 %{_sourcedir}/data/icons/hicolor/scalable/apps/com.rezkycodes.BatteryManager.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/
install -m 0644 %{_sourcedir}/data/icons/lucide/*.svg %{buildroot}%{_datadir}/%{name}/icons/lucide/
install -m 0644 %{_sourcedir}/data/systemd/battery-charge-threshold.service %{buildroot}%{_unitdir}/
install -m 0644 %{_sourcedir}/data/systemd/clamshell-server.conf %{buildroot}%{_sysconfdir}/systemd/logind.conf.d/
install -m 0644 %{_sourcedir}/data/udev/*.rules %{buildroot}%{_udevrulesdir}/
install -m 0644 %{_sourcedir}/data/sysctl/99-io-stability.conf %{buildroot}%{_sysctldir}/
install -m 0644 %{_sourcedir}/data/modprobe/nvidia-power-stability.conf %{buildroot}%{_prefix}/lib/modprobe.d/
install -m 0440 %{_sourcedir}/data/sudoers/asus-power-manager %{buildroot}%{_sysconfdir}/sudoers.d/asus-power-manager

%post
systemctl daemon-reload >/dev/null 2>&1 || :
systemctl enable --now battery-charge-threshold.service >/dev/null 2>&1 || :
udevadm control --reload-rules >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=power_supply >/dev/null 2>&1 || :
systemctl kill --kill-who=main --signal=HUP systemd-logind.service >/dev/null 2>&1 || :
update-desktop-database /usr/share/applications &> /dev/null || :
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor &>/dev/null || :

%preun
if [ $1 -eq 0 ]; then
    systemctl disable --now battery-charge-threshold.service >/dev/null 2>&1 || :
fi

%postun
systemctl daemon-reload >/dev/null 2>&1 || :
udevadm control --reload-rules >/dev/null 2>&1 || :
update-desktop-database /usr/share/applications &> /dev/null || :
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor &>/dev/null || :

%files
%{_bindir}/asus-power-manager
%{_libexecdir}/%{name}
%{_datadir}/applications/com.rezkycodes.BatteryManager.desktop
%{_datadir}/icons/hicolor/scalable/apps/com.rezkycodes.BatteryManager.svg
%{_datadir}/%{name}
%{_unitdir}/battery-charge-threshold.service
%config(noreplace) %{_sysconfdir}/systemd/logind.conf.d/clamshell-server.conf
%{_udevrulesdir}/*.rules
%{_sysctldir}/99-io-stability.conf
%{_prefix}/lib/modprobe.d/nvidia-power-stability.conf
%config(noreplace) %{_sysconfdir}/sudoers.d/asus-power-manager
