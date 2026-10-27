Name: nomo
Version: @VERSION@
Release: 1
Summary: Native Markdown editor
License: AGPL-3.0-or-later
URL: https://github.com/nomo-md/nomo
BuildArch: x86_64
Requires: fontconfig, freetype, libxkbcommon, libxkbcommon-x11, wayland-libs, vulkan-loader, libglvnd-egl, libglvnd-glx, libxcb, xcb-util-cursor, xdg-desktop-portal, xdg-utils
Recommends: google-noto-sans-cjk-fonts

%description
Local-first Markdown editor built with Rust and GPUI.

%install
mkdir -p %{buildroot}/usr
cp -a "@PAYLOAD@/usr/." %{buildroot}/usr/

%files
/usr/bin/nomo
/usr/share/applications/com.nomo.desktop.desktop
/usr/share/icons/hicolor/32x32/apps/nomo.png
/usr/share/icons/hicolor/128x128/apps/nomo.png
/usr/share/icons/hicolor/256x256/apps/nomo.png
/usr/share/nomo
%doc /usr/share/doc/nomo
