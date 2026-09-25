#!/bin/sh
# Install a JevGate release binary on Linux or macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/main/install.sh | sh
#
# JEVGATE_VERSION     version to install, such as 0.17.0 (default: the latest release)
# JEVGATE_INSTALL_DIR directory to install into (default: ~/.local/bin)
#
# The archive is checked against its published SHA-256 before anything is
# installed. On Windows, use `cargo binstall jevgate` or download the zip from
# the release page.
set -eu

repo="Tech-Byte-Frontier/jevgate"
install_dir="${JEVGATE_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    echo "jevgate install: $*" >&2
    exit 1
}

fetch() {
    if command -v curl > /dev/null 2>&1; then
        curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"
    elif command -v wget > /dev/null 2>&1; then
        wget -q --https-only -O "$2" "$1"
    else
        fail "curl or wget is needed"
    fi
}

sha256() {
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum > /dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "sha256sum or shasum is needed to check the download"
    fi
}

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64 | Linux-amd64) target=x86_64-unknown-linux-musl ;;
    Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    Darwin-arm64) target=aarch64-apple-darwin ;;
    *) fail "no release binary for $(uname -s) $(uname -m); build it with: cargo install jevgate --locked" ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

version="${JEVGATE_VERSION:-}"
if [ -z "$version" ]; then
    # github.com/…/releases/latest redirects to …/releases/tag/vX.Y.Z; unlike
    # the API, it has no rate limit for runners sharing an address.
    if command -v curl > /dev/null 2>&1; then
        latest=$(curl -fsSLI --proto '=https' -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") || latest=""
    else
        latest=$(wget -q --https-only -S --spider "https://github.com/$repo/releases/latest" 2>&1 | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1)
    fi
    version="${latest##*/tag/}"
    case "$latest" in */tag/v*) ;; *) version="" ;; esac
    [ -n "$version" ] || fail "could not find the latest release; set JEVGATE_VERSION"
fi
version="${version#v}"

archive="jevgate-$version-$target.tar.gz"
url="https://github.com/$repo/releases/download/v$version/$archive"
echo "Downloading JevGate $version for $target"
fetch "$url" "$tmp/$archive" || fail "could not download $url"
fetch "$url.sha256" "$tmp/$archive.sha256" || fail "could not download $url.sha256"

expected=$(cut -d ' ' -f 1 < "$tmp/$archive.sha256")
actual=$(sha256 "$tmp/$archive")
[ "$expected" = "$actual" ] || fail "checksum mismatch for $archive (expected $expected, got $actual)"

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$install_dir"
cp "$tmp/jevgate-$version-$target/jevgate" "$install_dir/jevgate.tmp"
chmod 755 "$install_dir/jevgate.tmp"
mv -f "$install_dir/jevgate.tmp" "$install_dir/jevgate"

echo "Installed $("$install_dir/jevgate" --version) to $install_dir/jevgate"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) echo "Add $install_dir to your PATH to run jevgate" ;;
esac
