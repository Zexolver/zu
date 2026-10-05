#!/usr/bin/env bash
# Builds zu_<version>_<arch>.deb for amd64 and arm64 into dist/.
#
# Static musl binaries are used on purpose: a glibc binary built on this machine would require a
# newer glibc than most current Debian/Ubuntu releases ship, defeating the point of a .deb. musl
# sidesteps that entirely.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=dist
rm -rf "$out"
mkdir -p "$out"

build_one() {
	local arch=$1 target=$2
	echo "==> $arch ($target)"
	rustup target add "$target" >/dev/null
	cargo build --release --target "$target"

	local pkg="$out/pkg-$arch"
	rm -rf "$pkg"
	install -Dm755 "target/$target/release/zu" "$pkg/usr/bin/zu"
	install -Dm644 "debian/sources.toml" "$pkg/etc/zu/sources.toml"
	install -Dm644 LICENSE "$pkg/usr/share/doc/zu/copyright" 2>/dev/null || true
	install -Dm644 README.md "$pkg/usr/share/doc/zu/README.md"

	local size_kb
	size_kb=$(du -sk --exclude=DEBIAN "$pkg" | cut -f1)
	mkdir -p "$pkg/DEBIAN"
	sed -e "s/@VERSION@/$version/" -e "s/@ARCH@/$arch/" -e "s/@SIZE@/$size_kb/" \
		debian/control.in >"$pkg/DEBIAN/control"
	cp debian/conffiles "$pkg/DEBIAN/conffiles"

	( cd "$pkg" && find . -path ./DEBIAN -prune -o -type f -print0 | \
		xargs -0 md5sum | sed 's#\./##' > DEBIAN/md5sums )

	local deb="$out/zu_${version}_${arch}.deb"
	( cd "$pkg/DEBIAN" && tar --owner=root --group=root --numeric-owner -cJf ../../control.tar.xz . )
	( cd "$pkg" && tar --owner=root --group=root --numeric-owner --exclude=DEBIAN -cJf ../data.tar.xz . )
	echo "2.0" >"$out/debian-binary"
	( cd "$out" && ar rc "$(basename "$deb")" debian-binary control.tar.xz data.tar.xz )
	rm -f "$out/debian-binary" "$out/control.tar.xz" "$out/data.tar.xz"
	echo "    -> $deb"
}

build_one amd64 x86_64-unknown-linux-musl
build_one arm64 aarch64-unknown-linux-musl

echo "==> done"
ls -la "$out"/*.deb
