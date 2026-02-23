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
    echo "  rpm    Build an RPM package"
    exit 1
}

get_version() {
    grep '^version' "$PROJECT_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

get_rpm_arch() {
    local machine
    machine="$(uname -m)"
    case "$machine" in
        x86_64)  echo "x86_64" ;;
        aarch64) echo "aarch64" ;;
        arm64)   echo "aarch64" ;;
        *)       echo "$machine" ;;
    esac
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

build_rpm() {
    local version
    version="$(get_version)"
    local arch
    arch="$(get_rpm_arch)"
    local pkg_name="nameroute-${version}-1.${arch}"

    echo "==> Building rpm package: $pkg_name"

    # Set up rpmbuild directory structure
    local rpmbuild_dir="$OUT_DIR/rpmbuild"
    rm -rf "$rpmbuild_dir"
    mkdir -p "$rpmbuild_dir"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

    # Copy sources
    cp "$PROJECT_ROOT/target/release/nameroute" "$rpmbuild_dir/SOURCES/nameroute"
    cp "$SCRIPT_DIR/systemd/nameroute.service" "$rpmbuild_dir/SOURCES/nameroute.service"
    cp "$PROJECT_ROOT/config.example.toml" "$rpmbuild_dir/SOURCES/config.toml"

    # Generate spec from template
    sed -e "s/%%VERSION%%/$version/g" \
        "$SCRIPT_DIR/rpm/nameroute.spec.tmpl" > "$rpmbuild_dir/SPECS/nameroute.spec"

    # Build RPM
    rpmbuild -bb \
        --define "_topdir $rpmbuild_dir" \
        --target "$arch" \
        "$rpmbuild_dir/SPECS/nameroute.spec"

    # Move RPM to output directory
    mkdir -p "$OUT_DIR"
    find "$rpmbuild_dir/RPMS" -name "*.rpm" -exec cp {} "$OUT_DIR/" \;

    # Clean rpmbuild directory
    rm -rf "$rpmbuild_dir"

    echo "==> Package created: $OUT_DIR/${pkg_name}.rpm"
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
    rpm)
        build_rpm
        ;;
    *)
        echo "Error: unknown format '$format'"
        usage
        ;;
esac
