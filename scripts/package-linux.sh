#!/usr/bin/env bash
set -euo pipefail

: "${VERSION:?Set VERSION to the Linux package version}"
format="${1:?Usage: package-linux.sh deb|rpm}"
root="$PWD"
stage="$root/target/installer-linux"
mkdir -p "$stage/usr/bin" "$stage/usr/share/applications" \
  "$stage/usr/share/doc/hydra-rust" "$root/dist"
install -m 755 target/release/hydra "$stage/usr/bin/hydra"
install -m 644 LICENSE README.md "$stage/usr/share/doc/hydra-rust/"
cat > "$stage/usr/share/applications/hydra-rust.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Hydra
Comment=Live-codable visual synthesizer
Exec=hydra
Terminal=false
Categories=AudioVideo;Graphics;
EOF

case "$format" in
  deb)
    mkdir -p "$stage/DEBIAN" "$stage/debian"
    cat > "$stage/debian/control" <<'EOF'
Source: hydra-rust
Section: graphics
Priority: optional
Maintainer: Hydra contributors <noreply@github.com>

Package: hydra-rust
Architecture: amd64
Description: Live-codable visual synthesizer
EOF
    dependencies="$(cd "$stage" && dpkg-shlibdeps -O -eusr/bin/hydra)"
    cat > "$stage/DEBIAN/control" <<EOF
Package: hydra-rust
Version: $VERSION
Section: graphics
Priority: optional
Architecture: amd64
Maintainer: Hydra contributors <noreply@github.com>
Depends: ${dependencies#shlibs:Depends=}, libegl1, libgl1, libx11-6, libxcursor1, libxi6, libxrandr2, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1, xdg-desktop-portal
Recommends: xdg-desktop-portal-gtk
Description: Live-codable visual synthesizer
 Hydra compiles Rhai scripts to GLSL and renders them with OpenGL.
EOF
    # Source metadata is only needed by dpkg-shlibdeps, not the installed app.
    rm "$stage/debian/control"
    rmdir "$stage/debian"
    dpkg-deb --root-owner-group --build "$stage" \
      "$root/dist/hydra-rust_${VERSION}_amd64.deb"
    dpkg-deb --info "$root/dist/hydra-rust_${VERSION}_amd64.deb"
    ;;
  rpm)
    top="$root/target/installer-rpm"
    mkdir -p "$top/BUILD" "$top/BUILDROOT" "$top/RPMS" \
      "$top/SOURCES" "$top/SPECS" "$top/SRPMS"
    cat > "$top/SPECS/hydra-rust.spec" <<EOF
Name: hydra-rust
Version: $VERSION
Release: 1
Summary: Live-codable visual synthesizer
License: AGPL-3.0-or-later
URL: https://github.com/sova-org/hydra-rust
Requires: mesa-libEGL, mesa-libGL, libX11, libXcursor, libXi, libXrandr
Requires: libxkbcommon, libxkbcommon-x11, wayland-libs, xdg-desktop-portal
Recommends: xdg-desktop-portal-gtk

%description
Hydra compiles Rhai scripts to GLSL and renders them with OpenGL.

%install
mkdir -p %{buildroot}
cp -a "$stage/usr" %{buildroot}/

%files
/usr/bin/hydra
/usr/share/applications/hydra-rust.desktop
%doc /usr/share/doc/hydra-rust/README.md
%license /usr/share/doc/hydra-rust/LICENSE
EOF
    rpmbuild --define "_topdir $top" --define "_build_id_links none" \
      --define "debug_package %{nil}" -bb "$top/SPECS/hydra-rust.spec"
    cp "$top"/RPMS/x86_64/*.rpm "$root/dist/"
    rpm -qip "$root"/dist/*.rpm
    ;;
  *)
    echo "Unsupported package format: $format" >&2
    exit 1
    ;;
esac
