#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUT_DIR="$SCRIPT_DIR/out"

# Ensure cargo is in PATH
if ! command -v cargo &>/dev/null && [[ -f "$HOME/.cargo/env" ]]; then
    source "$HOME/.cargo/env"
fi

usage() {
    echo "Usage: $0 <format>"
    echo ""
    echo "Formats:"
    echo "  deb    Build a Debian package"
    exit 1
}

get_version() {
    grep '^version' "$PROJECT_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

build_binary() {
    echo "==> Building release binary..."
    cargo build --release --manifest-path "$PROJECT_ROOT/Cargo.toml"
}

build_deb() {
    local version
    version="$(get_version)"
    local arch
    arch="$(dpkg --print-architecture)"
    local pkg_name="nameroute_${version}_${arch}"
    local staging="$OUT_DIR/$pkg_name"

    echo "==> Building deb package: $pkg_name"

    # Clean previous staging
    rm -rf "$staging"
    mkdir -p "$staging"

    # DEBIAN control files
    mkdir -p "$staging/DEBIAN"
    sed -e "s/%%VERSION%%/$version/g" \
        -e "s/%%ARCH%%/$arch/g" \
        "$SCRIPT_DIR/deb/control.tmpl" > "$staging/DEBIAN/control"
    cp "$SCRIPT_DIR/deb/conffiles" "$staging/DEBIAN/conffiles"
    cp "$SCRIPT_DIR/deb/postinst"  "$staging/DEBIAN/postinst"
    cp "$SCRIPT_DIR/deb/prerm"     "$staging/DEBIAN/prerm"
    cp "$SCRIPT_DIR/deb/postrm"    "$staging/DEBIAN/postrm"
    chmod 0755 "$staging/DEBIAN/postinst" "$staging/DEBIAN/prerm" "$staging/DEBIAN/postrm"

    # Binary
    mkdir -p "$staging/usr/bin"
    cp "$PROJECT_ROOT/target/release/nameroute" "$staging/usr/bin/nameroute"
    chmod 0755 "$staging/usr/bin/nameroute"

    # systemd unit
    mkdir -p "$staging/usr/lib/systemd/system"
    cp "$SCRIPT_DIR/systemd/nameroute.service" "$staging/usr/lib/systemd/system/nameroute.service"

    # Default config
    mkdir -p "$staging/etc/nameroute"
    cp "$PROJECT_ROOT/config.example.toml" "$staging/etc/nameroute/config.toml"

    # Build deb
    mkdir -p "$OUT_DIR"
    fakeroot dpkg-deb --build "$staging" "$OUT_DIR/${pkg_name}.deb"

    # Clean staging directory
    rm -rf "$staging"

    echo "==> Package created: $OUT_DIR/${pkg_name}.deb"
}

# --- Main ---

if [[ $# -lt 1 ]]; then
    usage
fi

format="$1"

build_binary

case "$format" in
    deb)
        build_deb
        ;;
    *)
        echo "Error: unknown format '$format'"
        usage
        ;;
esac
