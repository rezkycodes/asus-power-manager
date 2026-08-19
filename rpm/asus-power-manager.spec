Name:           asus-power-manager
Version:        2.0.0
Release:        1%{?dist}
Summary:        Tweaks ASUS TUF — system monitor & hardware control (Rust/GTK4)

License:        MIT
URL:            https://github.com/rezkycodes/asus-power-manager
BuildArch:      x86_64

Requires:       gtk4
Requires:       libadwaita
Requires:       upower
Requires:       systemd
Requires:       udev

Recommends:     tuned
Recommends:     hdparm
Recommends:     solaar

%global _bin_name asus-tuf-cpu
%global _appid com.rezkycodes.AsusTufCpu

%description
Native Rust/GTK4 + Libadwaita app for ASUS TUF Gaming laptops on Linux.
Realtime monitors for CPU, memory, GPU, fans, network, drives, battery,
processes and systemd services, plus hardware controls: battery charge limit
(80% health care), powersave/performance/auto CPU profiles, fan profiles,
GPU mode switching, keyboard RGB, and Logitech G304 tuning.

%install
rm -rf %{buildroot}
mkdir -p %{buildroot}%{_bindir}
mkdir -p %{buildroot}%{_libexecdir}/%{name}/scripts
mkdir -p %{buildroot}%{_datadir}/applications
mkdir -p %{buildroot}%{_datadir}/icons/hicolor/scalable/apps
mkdir -p %{buildroot}%{_datadir}/%{_bin_name}/icons
mkdir -p %{buildroot}%{_unitdir}
mkdir -p %{buildroot}%{_udevrulesdir}
mkdir -p %{buildroot}%{_sysctldir}
mkdir -p %{buildroot}%{_prefix}/lib/modprobe.d
mkdir -p %{buildroot}%{_sysconfdir}/systemd/logind.conf.d
mkdir -p %{buildroot}%{_sysconfdir}/sudoers.d

# Rust binary + backward-compat symlink for the old launcher name
install -m 0755 %{_sourcedir}/bin/%{_bin_name} %{buildroot}%{_bindir}/%{_bin_name}
ln -sf %{_bin_name} %{buildroot}%{_bindir}/asus-power-manager

install -m 0755 %{_sourcedir}/scripts/* %{buildroot}%{_libexecdir}/%{name}/scripts/
install -m 0644 %{_sourcedir}/data/%{_appid}.desktop %{buildroot}%{_datadir}/applications/
install -m 0644 %{_sourcedir}/tweak-asus-tuf.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{_appid}.svg
install -m 0644 %{_sourcedir}/lucide-icons/*.svg %{buildroot}%{_datadir}/%{_bin_name}/icons/
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
%{_bindir}/%{_bin_name}
%{_bindir}/asus-power-manager
%{_libexecdir}/%{name}
%{_datadir}/applications/%{_appid}.desktop
%{_datadir}/icons/hicolor/scalable/apps/%{_appid}.svg
%{_datadir}/%{_bin_name}
%{_unitdir}/battery-charge-threshold.service
%config(noreplace) %{_sysconfdir}/systemd/logind.conf.d/clamshell-server.conf
%{_udevrulesdir}/*.rules
%{_sysctldir}/99-io-stability.conf
%{_prefix}/lib/modprobe.d/nvidia-power-stability.conf
%config(noreplace) %{_sysconfdir}/sudoers.d/asus-power-manager
