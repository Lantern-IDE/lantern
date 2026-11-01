#!/bin/sh
# Lantern CLI installer for macOS and Linux. No root needed.
#
#   curl -fsSL -o install.sh https://raw.githubusercontent.com/Lantern-IDE/lantern/main/scripts/install.sh
#   sh install.sh
#
# Finds the newest cli-v* release through the GitHub API ("latest" belongs to the desktop app),
# downloads the archive and SHA256SUMS to a temp folder, verifies the checksum, and only then installs.
#
# Environment variables:
#   LANTERN_VERSION       install this version (0.1.0 or cli-v0.1.0) and skip the API lookup
#   LANTERN_INSTALL_DIR   install folder (default: ~/.local/bin)
#   LANTERN_INSTALL_BASE  https:// URL or local folder to use instead of GitHub. It must hold
#                         releases.json and download/<tag>/<file>. For mirrors and tests.
set -eu

REPO=Lantern-IDE/lantern

die() {
  printf 'Lantern install failed: %s\n' "$*" >&2
  exit 1
}

has() {
  command -v "$1" >/dev/null 2>&1
}

fetch() { # <source> <dest>
  case "$1" in
    https://*)
      if has curl; then
        curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 --output "$2" "$1"
      elif has wget; then
        wget -q -O "$2" "$1"
      else
        die "curl or wget is required."
      fi
      ;;
    *://*)
      die "refusing to download from $1: only https:// or a local folder is allowed."
      ;;
    *)
      cp "$1" "$2"
      ;;
  esac
}

sha256_of() {
  if has sha256sum; then
    sha256sum "$1" | awk '{print $1}'
  elif has shasum; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif has openssl; then
    openssl dgst -sha256 "$1" | awk '{print $NF}'
  else
    die "sha256sum, shasum or openssl is required to verify the download."
  fi
}

uses_musl() {
  if ldd --version 2>&1 | grep -qi musl; then
    return 0
  fi
  ls /lib/ld-musl-* >/dev/null 2>&1
}

detect_target() {
  case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) die "no Lantern CLI build for this architecture: $(uname -m)." ;;
  esac
  case "$(uname -s)" in
    Darwin)
      # A terminal under Rosetta reports x86_64 on Apple Silicon. Take the native build.
      if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ]; then
        arch=aarch64
      fi
      echo "$arch-apple-darwin"
      ;;
    Linux)
      if uses_musl; then
        die "the Linux builds need glibc, and this system uses musl (Alpine?). To build from source: cargo install --git https://github.com/$REPO lantern-context --locked"
      fi
      echo "$arch-unknown-linux-gnu"
      ;;
    MINGW* | MSYS* | CYGWIN*)
      die "on Windows, use scripts/install.ps1."
      ;;
    *)
      die "no Lantern CLI build for this system: $(uname -s)."
      ;;
  esac
}

newest_tag() { # <releases.json>
  # Anonymous API calls never list drafts. The body text cannot match: its quotes are escaped.
  version=$(grep -o '"tag_name": *"cli-v[0-9]*\.[0-9]*\.[0-9]*"' "$1" \
    | sed 's/.*"cli-v//; s/"$//' \
    | sort -t. -k1,1n -k2,2n -k3,3n \
    | tail -n 1) || true
  [ -n "$version" ] || die "no published cli-v* release found at $list_source."
  echo "cli-v$version"
}

main() {
  base=${LANTERN_INSTALL_BASE:-}
  base=${base%/}
  if [ -n "$base" ]; then
    list_source=$base/releases.json
    asset_base=$base/download
  else
    list_source="https://api.github.com/repos/$REPO/releases?per_page=100"
    asset_base=https://github.com/$REPO/releases/download
  fi
  if [ -n "${LANTERN_INSTALL_DIR:-}" ]; then
    dir=$LANTERN_INSTALL_DIR
  else
    [ -n "${HOME:-}" ] || die "HOME is not set. Set LANTERN_INSTALL_DIR to choose a folder."
    dir=$HOME/.local/bin
  fi
  target=$(detect_target)

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 1' HUP INT TERM

  if [ -n "${LANTERN_VERSION:-}" ]; then
    version=${LANTERN_VERSION#cli-}
    version=${version#v}
    case "$version" in
      *[!0-9.]* | "" | .* | *. | *..*) die "LANTERN_VERSION must look like 0.1.0, got '$LANTERN_VERSION'." ;;
    esac
    tag=cli-v$version
  else
    hint=
    [ -n "$base" ] || hint=" The GitHub API allows 60 anonymous requests per hour. If you hit that limit, wait or set LANTERN_VERSION to skip this lookup."
    fetch "$list_source" "$tmp/releases.json" || die "could not read $list_source.$hint"
    tag=$(newest_tag "$tmp/releases.json")
    version=${tag#cli-v}
  fi
  echo "Installing Lantern CLI $version"

  name=lantern-$target.tar.gz
  fetch "$asset_base/$tag/SHA256SUMS" "$tmp/SHA256SUMS" \
    || die "could not download SHA256SUMS for $tag."
  want=$(tr -d '\r' <"$tmp/SHA256SUMS" | awk -v f="$name" '{ n = $2; sub(/^\*/, "", n); if (n == f) { print tolower($1); exit } }')
  [ -n "$want" ] || die "release $tag has no build for this system ($target)."

  fetch "$asset_base/$tag/$name" "$tmp/$name" || die "could not download $name."
  got=$(sha256_of "$tmp/$name")
  if [ "$got" != "$want" ]; then
    die "checksum mismatch for $name.
  SHA256SUMS: $want
  downloaded: $got
Nothing was installed. Try again, and report it if this keeps happening."
  fi
  echo "Checksum OK ($name)"

  mkdir "$tmp/unpacked"
  tar -xzf "$tmp/$name" -C "$tmp/unpacked" || die "could not unpack $name."
  [ -f "$tmp/unpacked/lantern" ] || die "$name does not contain lantern."

  # Run the new binary before touching an existing install. It runs from the install folder
  # because the temp folder may be mounted noexec.
  mkdir -p "$dir" || die "could not create $dir."
  staged=$dir/lantern.new.$$
  cp "$tmp/unpacked/lantern" "$staged" || die "could not write to $dir."
  chmod 755 "$staged"
  reported=$("$staged" --version 2>&1) || {
    rm -f "$staged"
    die "the downloaded binary does not run on this system: $reported
On Linux this usually means glibc is older than 2.34. To build from source: cargo install --git https://github.com/$REPO lantern-context --locked"
  }
  if [ "$reported" != "lantern $version" ]; then
    rm -f "$staged"
    die "the downloaded binary reported '$reported', expected 'lantern $version'."
  fi
  mv -f "$staged" "$dir/lantern" || die "could not replace $dir/lantern."
  echo "Installed $dir/lantern"

  case ":${PATH:-}:" in
    *":$dir:"*)
      found=$(command -v lantern 2>/dev/null || true)
      if [ -n "$found" ] && [ "$found" != "$dir/lantern" ]; then
        echo "Note: 'lantern' currently runs $found, which comes earlier on your PATH."
      fi
      ;;
    *)
      case "${SHELL:-}" in
        */zsh) rc=~/.zshrc ;;
        */bash) rc=~/.bashrc ;;
        *) rc=~/.profile ;;
      esac
      echo
      echo "$dir is not on your PATH. To add it, run this and open a new terminal:"
      echo "  echo 'export PATH=\"$dir:\$PATH\"' >> $rc"
      ;;
  esac
}

main "$@"
