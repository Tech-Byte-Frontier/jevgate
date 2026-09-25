#!/usr/bin/env bash
# Print the Homebrew formula for a release: homebrew-formula.sh VERSION SHA256SUMS
# The release workflow writes it to Tech-Byte-Frontier/homebrew-tap.
set -euo pipefail

version="$1"
sums="$2"
base="https://github.com/Tech-Byte-Frontier/jevgate/releases/download/v$version"

sha() {
    local archive="jevgate-$version-$1.tar.gz"
    local hash
    hash=$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' "$sums")
    [ -n "$hash" ] || { echo "No checksum for $archive in $sums" >&2; exit 1; }
    echo "$hash"
}

cat <<FORMULA
class Jevgate < Formula
  desc "Code-review gate that asks TypeSafe Jev small questions about your code"
  homepage "https://github.com/Tech-Byte-Frontier/jevgate"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "$base/jevgate-$version-aarch64-apple-darwin.tar.gz"
      sha256 "$(sha aarch64-apple-darwin)"
    end
    on_intel do
      url "$base/jevgate-$version-x86_64-apple-darwin.tar.gz"
      sha256 "$(sha x86_64-apple-darwin)"
    end
  end

  on_linux do
    on_arm do
      url "$base/jevgate-$version-aarch64-unknown-linux-musl.tar.gz"
      sha256 "$(sha aarch64-unknown-linux-musl)"
    end
    on_intel do
      url "$base/jevgate-$version-x86_64-unknown-linux-musl.tar.gz"
      sha256 "$(sha x86_64-unknown-linux-musl)"
    end
  end

  def install
    bin.install "jevgate"
  end

  test do
    assert_match "jevgate #{version}", shell_output("#{bin}/jevgate --version")
    system bin/"jevgate", "rules"
  end
end
FORMULA
