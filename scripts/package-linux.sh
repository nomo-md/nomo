#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || { echo 'Run on x86_64 Linux.' >&2; exit 1; }
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target=x86_64-unknown-linux-gnu
metadata="$(cargo metadata --locked --no-deps --format-version 1)"
version="$(jq -r '.packages[] | select(.name == "nomo-gpui") | .version' <<< "$metadata")"
package_version="${version//-/~}"
target_dir="$(jq -r '.target_directory' <<< "$metadata")"
if [[ "${1:-}" != --skip-build ]]; then
  cargo build --locked --release --target "$target" --bin nomo
fi
binary="$target_dir/$target/release/nomo"
file "$binary" | grep -q 'ELF 64-bit.*x86-64'
if ldd "$binary" | grep 'not found'; then echo 'Missing shared libraries' >&2; exit 1; fi
output="$root/.artifacts/packages"
mkdir -p "$output" "$root/.artifacts/package-work"
stage="$(mktemp -d "$root/.artifacts/package-work/linux.XXXXXX")"
payload="$stage/payload"
mkdir -p "$payload/usr/bin" "$payload/usr/share/applications" "$payload/usr/share/doc/nomo" "$payload/usr/share/nomo/samples/assets"
install -m755 "$binary" "$payload/usr/bin/nomo"
install -m644 scripts/packaging/nomo.desktop "$payload/usr/share/applications/com.nomo.desktop.desktop"
install -m644 LICENSE "$payload/usr/share/doc/nomo/copyright"
install -m644 src-gpui/assets/icons/LICENSE "$payload/usr/share/doc/nomo/LICENSE-Lucide"
install -m644 src-gpui/assets/licenses/LICENSE-STIX "$payload/usr/share/doc/nomo/LICENSE-STIX"
install -m644 src-gpui/assets/licenses/LICENSE-Rendering "$payload/usr/share/doc/nomo/LICENSE-Rendering"
install -m644 src-gpui/assets/licenses/LICENSE-Export "$payload/usr/share/doc/nomo/LICENSE-Export"
install -m644 sample.md "$payload/usr/share/nomo/samples/"
install -m644 assets/128x128.png "$payload/usr/share/nomo/samples/assets/"
for entry in '32:32x32.png' '128:128x128.png' '256:128x128@2x.png'; do
  size="${entry%%:*}"
  mkdir -p "$payload/usr/share/icons/hicolor/${size}x${size}/apps"
  install -m644 "src-gpui/assets/app/${entry#*:}" "$payload/usr/share/icons/hicolor/${size}x${size}/apps/nomo.png"
done
desktop-file-validate "$payload/usr/share/applications/com.nomo.desktop.desktop"
# dpkg-shlibdeps 扫描直接链接；dlopen 的图形和字体库必须另外声明。
mkdir -p "$stage/debian" "$payload/DEBIAN"
printf 'Source: nomo\nSection: editors\nPriority: optional\nMaintainer: Nomo maintainers <noreply@github.com>\n\nPackage: nomo\nArchitecture: amd64\nDescription: Native Markdown editor\n' > "$stage/debian/control"
linked="$(cd "$stage" && dpkg-shlibdeps -O -e"$binary" | sed -n 's/^shlibs:Depends=//p')"
runtime='libfontconfig1, libfreetype6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1, libvulkan1, libegl1, libgl1, libxcb1, libxcb-xkb1, libxcb-randr0, libxcb-xinput0, libxcb-cursor0, xdg-desktop-portal, xdg-utils'
cat > "$payload/DEBIAN/control" <<EOF
Package: nomo
Version: $package_version
Section: editors
Priority: optional
Architecture: amd64
Maintainer: Nomo maintainers <noreply@github.com>
Depends: ${linked:+$linked, }$runtime
Recommends: fonts-noto-cjk
Homepage: https://github.com/nomo-md/nomo
Description: Native Markdown editor built with Rust and GPUI
EOF
dpkg-deb --root-owner-group --build "$payload" "$output/Nomo_${version}_amd64.deb"
dpkg-deb --info "$output/Nomo_${version}_amd64.deb"
rpmroot="$stage/rpm"
mkdir -p "$rpmroot/BUILD" "$rpmroot/BUILDROOT" "$rpmroot/RPMS" "$rpmroot/SOURCES" "$rpmroot/SPECS" "$rpmroot/SRPMS"
# %files 明确列举应用文件，不拥有 /usr 等系统目录。
sed -e "s/@VERSION@/$package_version/g" -e "s|@PAYLOAD@|$payload|g" scripts/packaging/nomo.spec > "$rpmroot/SPECS/nomo.spec"
rpmbuild -bb --define "_topdir $rpmroot" --define '_build_id_links none' --define '__os_install_post %{nil}' "$rpmroot/SPECS/nomo.spec"
rpmfile="$rpmroot/RPMS/x86_64/nomo-${package_version}-1.x86_64.rpm"
rpm -qip "$rpmfile"
cp "$rpmfile" "$output/Nomo-${version}-1.x86_64.rpm"
echo "Linux packages: $output"
