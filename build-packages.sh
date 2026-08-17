#!/usr/bin/env bash
# ==============================================================================
# build-packages.sh — Build .deb and .rpm packages for ASUS Power Manager
# ==============================================================================

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
DIST_DIR="$ROOT_DIR/dist"
BUILD_DIR="$ROOT_DIR/build"
PKG_NAME="asus-power-manager"
PKG_VER="1.0.0"

echo "=== Building ASUS Power Manager Packages (v$PKG_VER) ==="
rm -rf "$DIST_DIR" "$BUILD_DIR"
mkdir -p "$DIST_DIR" "$BUILD_DIR"

# ─────────────────────────────────────────────
# 1. Build .deb Package
# ─────────────────────────────────────────────
echo ""
echo "[1/2] Building Debian/Ubuntu package (.deb)..."
DEB_ROOT="$BUILD_DIR/deb_root"
mkdir -p "$DEB_ROOT/DEBIAN"
mkdir -p "$DEB_ROOT/usr/bin"
mkdir -p "$DEB_ROOT/usr/libexec/$PKG_NAME/scripts"
mkdir -p "$DEB_ROOT/usr/share/applications"
mkdir -p "$DEB_ROOT/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$DEB_ROOT/lib/systemd/system"
mkdir -p "$DEB_ROOT/etc/systemd/logind.conf.d"
mkdir -p "$DEB_ROOT/lib/udev/rules.d"
mkdir -p "$DEB_ROOT/etc/sysctl.d"
mkdir -p "$DEB_ROOT/lib/modprobe.d"
mkdir -p "$DEB_ROOT/etc/sudoers.d"

# Copy files into deb root
cp "$ROOT_DIR/src/asus-power-manager" "$DEB_ROOT/usr/bin/"
chmod 0755 "$DEB_ROOT/usr/bin/asus-power-manager"

cp "$ROOT_DIR/scripts/"* "$DEB_ROOT/usr/libexec/$PKG_NAME/scripts/"
chmod 0755 "$DEB_ROOT/usr/libexec/$PKG_NAME/scripts/"*

cp "$ROOT_DIR/data/com.rezkycodes.BatteryManager.desktop" "$DEB_ROOT/usr/share/applications/"
cp "$ROOT_DIR/data/icons/hicolor/scalable/apps/com.rezkycodes.BatteryManager.svg" "$DEB_ROOT/usr/share/icons/hicolor/scalable/apps/"
cp "$ROOT_DIR/data/systemd/battery-charge-threshold.service" "$DEB_ROOT/lib/systemd/system/"
cp "$ROOT_DIR/data/systemd/clamshell-server.conf" "$DEB_ROOT/etc/systemd/logind.conf.d/"
cp "$ROOT_DIR/data/udev/"*.rules "$DEB_ROOT/lib/udev/rules.d/"
cp "$ROOT_DIR/data/sysctl/99-io-stability.conf" "$DEB_ROOT/etc/sysctl.d/"
cp "$ROOT_DIR/data/modprobe/nvidia-power-stability.conf" "$DEB_ROOT/lib/modprobe.d/"
cp "$ROOT_DIR/data/sudoers/asus-power-manager" "$DEB_ROOT/etc/sudoers.d/"
chmod 0440 "$DEB_ROOT/etc/sudoers.d/asus-power-manager"

# Copy debian control scripts
cp "$ROOT_DIR/debian/control" "$DEB_ROOT/DEBIAN/"
cp "$ROOT_DIR/debian/postinst" "$DEB_ROOT/DEBIAN/"
cp "$ROOT_DIR/debian/prerm" "$DEB_ROOT/DEBIAN/"
cp "$ROOT_DIR/debian/postrm" "$DEB_ROOT/DEBIAN/"
chmod 0755 "$DEB_ROOT/DEBIAN/postinst" "$DEB_ROOT/DEBIAN/prerm" "$DEB_ROOT/DEBIAN/postrm"

# Build DEB
dpkg-deb --build --root-owner-group "$DEB_ROOT" "$DIST_DIR/${PKG_NAME}_${PKG_VER}_all.deb"
echo "✓ Created: $DIST_DIR/${PKG_NAME}_${PKG_VER}_all.deb"

# ─────────────────────────────────────────────
# 2. Build .rpm Package
# ─────────────────────────────────────────────
echo ""
echo "[2/2] Building Fedora/RHEL/openSUSE package (.rpm)..."
RPM_TOP="$BUILD_DIR/rpmbuild"
mkdir -p "$RPM_TOP"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Copy sources for rpmbuild
cp -r "$ROOT_DIR/src" "$RPM_TOP/SOURCES/"
cp -r "$ROOT_DIR/scripts" "$RPM_TOP/SOURCES/"
cp -r "$ROOT_DIR/data" "$RPM_TOP/SOURCES/"
cp "$ROOT_DIR/rpm/$PKG_NAME.spec" "$RPM_TOP/SPECS/"

rpmbuild --define "_topdir $RPM_TOP" -bb "$RPM_TOP/SPECS/$PKG_NAME.spec"
cp "$RPM_TOP"/RPMS/noarch/*.rpm "$DIST_DIR/"
echo "✓ Created RPM package in $DIST_DIR/"

echo ""
echo "=== BUILD COMPLETE ==="
ls -lh "$DIST_DIR"
