#!/bin/sh
# zencode installer.
#
#   curl -fsSL https://raw.githubusercontent.com/team-mirae/zencode/main/install.sh | sh
#
# Installs the latest release for this machine into ~/.local/bin, with no
# sudo needed. Environment variables:
#
#   ZENCODE_VERSION=v0.1.0      install that release instead of the latest
#   ZENCODE_INSTALL_DIR=/path   install somewhere other than ~/.local/bin

set -eu

REPO="team-mirae/zencode"
VERSION="${ZENCODE_VERSION:-latest}"
INSTALL_DIR="${ZENCODE_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'zencode: %s\n' "$*"; }
die() { printf 'zencode: error: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
    Linux) os="unknown-linux-musl" ;;
    Darwin) os="apple-darwin" ;;
    *) die "no prebuilt binary for $(uname -s); see https://github.com/$REPO/releases" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch="x86_64" ;;
    aarch64 | arm64) arch="aarch64" ;;
    *) die "no prebuilt binary for $(uname -m); see https://github.com/$REPO/releases" ;;
esac
# A shell running under Rosetta reports x86_64; install the native build.
if [ "$os" = "apple-darwin" ] && [ "$arch" = "x86_64" ] &&
    [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = "1" ]; then
    arch="aarch64"
fi
target="$arch-$os"
asset="zencode-$target.tar.gz"

if [ "$VERSION" = "latest" ]; then
    base="https://github.com/$REPO/releases/latest/download"
else
    base="https://github.com/$REPO/releases/download/$VERSION"
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "curl or wget is required"
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    die "sha256sum or shasum is required to verify the download"
fi

tmp="$(mktemp -d 2>/dev/null || mktemp -d -t zencode)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $asset ($VERSION)"
fetch "$base/$asset" "$tmp/$asset" || die "download failed: $base/$asset"
fetch "$base/$asset.sha256" "$tmp/$asset.sha256" || die "checksum download failed"

expected="$(cut -d ' ' -f 1 <"$tmp/$asset.sha256")"
actual="$(sha256 "$tmp/$asset")"
[ "$expected" = "$actual" ] || die "checksum mismatch for $asset"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$INSTALL_DIR"
cp "$tmp/zencode-$target/zencode" "$INSTALL_DIR/zencode.tmp"
chmod 755 "$INSTALL_DIR/zencode.tmp"
# Rename into place so a running zencode isn't overwritten mid-flight.
mv -f "$INSTALL_DIR/zencode.tmp" "$INSTALL_DIR/zencode"

say "installed $("$INSTALL_DIR/zencode" --version) to $INSTALL_DIR/zencode"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        say "$INSTALL_DIR is not on your PATH. Add this to your shell profile:"
        printf '\n    export PATH="%s:$PATH"\n\n' "$INSTALL_DIR"
        ;;
esac
