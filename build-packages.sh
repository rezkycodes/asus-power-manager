#!/usr/bin/env bash
# ==============================================================================
# build-packages.sh — Build .deb and .rpm packages for Tweaks ASUS TUF (Rust)
# ==============================================================================

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
DIST_DIR="$ROOT_DIR/dist"
BUILD_DIR="$ROOT_DIR/build"
PKG_NAME="asus-power-manager"      # keep: libexec path + sudoers wildcard depend on it
BIN_NAME="asus-tuf-cpu"            # Rust binary name (also the icon resolver dir)
APP_ID="com.rezkycodes.AsusTufCpu"
PKG_VER="2.0.0"

echo "=== Building Tweaks ASUS TUF (Rust) Packages (v$PKG_VER) ==="

# ─────────────────────────────────────────────
# 0. Compile the Rust release binary
# ─────────────────────────────────────────────
echo ""
echo "[0/2] Compiling Rust release binary..."
( cd "$ROOT_DIR/rust-gui" && cargo build --release )
RUST_BIN="$ROOT_DIR/rust-gui/target/release/$BIN_NAME"
test -x "$RUST_BIN" || { echo "ERROR: Rust binary not found at $RUST_BIN"; exit 1; }

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
mkdir -p "$DEB_ROOT/usr/share/$BIN_NAME/icons"
mkdir -p "$DEB_ROOT/lib/systemd/system"
mkdir -p "$DEB_ROOT/etc/systemd/logind.conf.d"
mkdir -p "$DEB_ROOT/lib/udev/rules.d"
mkdir -p "$DEB_ROOT/etc/sysctl.d"
mkdir -p "$DEB_ROOT/lib/modprobe.d"
mkdir -p "$DEB_ROOT/etc/sudoers.d"

# Rust binary + backward-compat symlink for the old launcher name
install -m 0755 "$RUST_BIN" "$DEB_ROOT/usr/bin/$BIN_NAME"
ln -sf "$BIN_NAME" "$DEB_ROOT/usr/bin/asus-power-manager"

# Hardware backend scripts (called via sudo -n by the app)
cp "$ROOT_DIR/scripts/"* "$DEB_ROOT/usr/libexec/$PKG_NAME/scripts/"
chmod 0755 "$DEB_ROOT/usr/libexec/$PKG_NAME/scripts/"*

# Desktop entry + brand icon + bundled lucide icons
cp "$ROOT_DIR/data/$APP_ID.desktop" "$DEB_ROOT/usr/share/applications/"
install -m 0644 "$ROOT_DIR/tweak-asus-tuf.svg" "$DEB_ROOT/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
cp "$ROOT_DIR/rust-gui/icons/"*.svg "$DEB_ROOT/usr/share/$BIN_NAME/icons/"

# System stability assets
cp "$ROOT_DIR/data/systemd/battery-charge-threshold.service" "$DEB_ROOT/lib/systemd/system/"
cp "$ROOT_DIR/data/systemd/clamshell-server.conf" "$DEB_ROOT/etc/systemd/logind.conf.d/"
cp "$ROOT_DIR/data/udev/"*.rules "$DEB_ROOT/lib/udev/rules.d/"
cp "$ROOT_DIR/data/sysctl/99-io-stability.conf" "$DEB_ROOT/etc/sysctl.d/"
cp "$ROOT_DIR/data/modprobe/nvidia-power-stability.conf" "$DEB_ROOT/lib/modprobe.d/"
cp "$ROOT_DIR/data/sudoers/asus-power-manager" "$DEB_ROOT/etc/sudoers.d/"
chmod 0440 "$DEB_ROOT/etc/sudoers.d/asus-power-manager"

# Debian control scripts
cp "$ROOT_DIR/debian/control" "$DEB_ROOT/DEBIAN/"
cp "$ROOT_DIR/debian/postinst" "$DEB_ROOT/DEBIAN/"
cp "$ROOT_DIR/debian/prerm" "$DEB_ROOT/DEBIAN/"
cp "$ROOT_DIR/debian/postrm" "$DEB_ROOT/DEBIAN/"
chmod 0755 "$DEB_ROOT/DEBIAN/postinst" "$DEB_ROOT/DEBIAN/prerm" "$DEB_ROOT/DEBIAN/postrm"

dpkg-deb --build --root-owner-group "$DEB_ROOT" "$DIST_DIR/${PKG_NAME}_${PKG_VER}_amd64.deb"
echo "✓ Created: $DIST_DIR/${PKG_NAME}_${PKG_VER}_amd64.deb"

# ─────────────────────────────────────────────
# 2. Build .rpm Package
# ─────────────────────────────────────────────
echo ""
echo "[2/2] Building Fedora/RHEL/openSUSE package (.rpm)..."
RPM_TOP="$BUILD_DIR/rpmbuild"
mkdir -p "$RPM_TOP"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Stage prebuilt binary + assets into SOURCES
mkdir -p "$RPM_TOP/SOURCES/bin"
install -m 0755 "$RUST_BIN" "$RPM_TOP/SOURCES/bin/$BIN_NAME"
cp -r "$ROOT_DIR/scripts" "$RPM_TOP/SOURCES/"
cp -r "$ROOT_DIR/data" "$RPM_TOP/SOURCES/"
cp -r "$ROOT_DIR/rust-gui/icons" "$RPM_TOP/SOURCES/lucide-icons"
cp "$ROOT_DIR/tweak-asus-tuf.svg" "$RPM_TOP/SOURCES/"
cp "$ROOT_DIR/rpm/$PKG_NAME.spec" "$RPM_TOP/SPECS/"

rpmbuild --define "_topdir $RPM_TOP" -bb "$RPM_TOP/SPECS/$PKG_NAME.spec"
cp "$RPM_TOP"/RPMS/*/*.rpm "$DIST_DIR/"
echo "✓ Created RPM package in $DIST_DIR/"

echo ""
echo "=== BUILD COMPLETE ==="
ls -lh "$DIST_DIR"
