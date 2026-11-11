#!/usr/bin/env bash
# Install reviewed cargo-dist bytes, never a remotely supplied installer script.
set -euo pipefail

# Digests verified against the publisher's v0.31.0 release asset metadata and
# SHA-256 sidecars: https://github.com/axodotdev/cargo-dist/releases/tag/v0.31.0
cargo_dist_asset() {
  case "$1/$2" in
    Linux/x86_64)
      printf '%s\n' 'x86_64-unknown-linux-musl 1d46b93a785e8b666c0e7add581d747d53c3f29589cc3ef9aee60341ad899023 tar.xz' ;;
    Linux/aarch64|Linux/arm64)
      printf '%s\n' 'aarch64-unknown-linux-musl 2ce8104f4e10cbcb4b46eb892dd17b84237d8477f2de7aa024ce5a897447a489 tar.xz' ;;
    Darwin/x86_64)
      printf '%s\n' 'x86_64-apple-darwin fd4d8f9f07802359cbcdc52bac3abd7d5201c4b73a7cbcdd6faca2232a389f0c tar.xz' ;;
    Darwin/arm64|Darwin/aarch64)
      printf '%s\n' 'aarch64-apple-darwin decb01c64c12501931c3cac3111b368a7f48adf8d9e65455c08e5757b9a1fd6f tar.xz' ;;
    MINGW*/x86_64|MSYS*/x86_64)
      printf '%s\n' 'x86_64-pc-windows-msvc a14e17557b269b101405e0cc6b647581d56313c954a51c7fddd423bba21e17b2 zip' ;;
    *) echo "Unsupported cargo-dist host: $1/$2" >&2; return 1 ;;
  esac
}

verify_sha256() {
  local expected=$1 file=$2 actual
  if command -v sha256sum > /dev/null 2>&1; then
    actual=$(sha256sum "$file")
  else
    actual=$(shasum -a 256 "$file")
  fi
  actual=${actual%% *}
  if [[ $actual != "$expected" ]]; then
    echo "cargo-dist SHA-256 mismatch; refusing to extract or execute downloaded bytes" >&2
    return 1
  fi
}

install_cargo_dist() (
  local target expected extension archive binary work_dir install_dir
  read -r target expected extension < <(cargo_dist_asset "$(uname -s)" "$(uname -m)")
  work_dir=$(mktemp -d)
  archive="$work_dir/cargo-dist.$extension"
  binary=dist
  [[ $extension != zip ]] || binary=dist.exe
  # Only these two files are created in the private temporary directory.
  trap 'rm -f "$archive" "$work_dir/$binary"; rmdir "$work_dir"' EXIT
  curl --proto '=https' --tlsv1.2 -fL --retry 6 --retry-all-errors --retry-delay 5 \
    "https://github.com/axodotdev/cargo-dist/releases/download/v0.31.0/cargo-dist-$target.$extension" \
    --output "$archive"
  verify_sha256 "$expected" "$archive"
  if [[ $extension == zip ]]; then
    unzip -p "$archive" "$binary" > "$work_dir/$binary"
  else
    tar -xJOf "$archive" "cargo-dist-$target/$binary" > "$work_dir/$binary"
  fi
  test -s "$work_dir/$binary"
  install_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
  mkdir -p "$install_dir"
  cp "$work_dir/$binary" "$install_dir/$binary"
  chmod +x "$install_dir/$binary"
  if [[ -n ${GITHUB_PATH:-} ]]; then
    printf '%s\n' "$install_dir" >> "$GITHUB_PATH"
  fi
  "$install_dir/$binary" --version
)

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  install_cargo_dist
fi
