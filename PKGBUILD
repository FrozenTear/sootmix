# Maintainer: SootMix Contributors
pkgname=sootmix
pkgver=0.1.0
pkgrel=1
pkgdesc="Audio routing and mixing application for Linux using PipeWire"
arch=('x86_64')
url="https://github.com/FrozenTear/sootmix"
license=('MPL-2.0')
depends=('pipewire' 'libpipewire' 'dbus')
makedepends=('rust' 'cargo' 'clang')
optdepends=('pipewire-pulse: PulseAudio compatibility')
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
    cd "$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    make build
}

package() {
    cd "$pkgname-$pkgver"
    make DESTDIR="$pkgdir" install
}
