#!/usr/bin/env bash
# Cache only the versioned tool. Advisory data is fetched by the subsequent audit.
set -euo pipefail
version=${1:?expected cargo-audit version}
install_root=${2:?isolated installation root}
cache_hit=${3:-false}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 1
binary="$install_root/bin/cargo-audit"
if [[ -f $binary && $cache_hit != true ]]; then
  echo 'Refusing a cargo-audit executable from a non-exact cache restore' >&2
  exit 1
fi
if [[ ! -f $binary ]]; then
  cargo install --locked --version "=$version" --root "$install_root" cargo-audit
fi
actual=$("$binary" --version)
if [[ $actual != "cargo-audit $version" ]]; then
  echo "Unexpected cached cargo-audit version: $actual" >&2
  exit 1
fi
printf 'Verified cargo-audit %s\n' "$version"
