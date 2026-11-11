#!/usr/bin/env bash
# Exercise rejection before extraction/execution without accessing the network.
set -euo pipefail
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/install-cargo-dist.sh
source "$script_dir/install-cargo-dist.sh"

work_dir=$(mktemp -d)
trap 'rm -f "$work_dir/bin/"* "$work_dir/"*.log "$work_dir/fixture"; rmdir "$work_dir/bin" "$work_dir/cargo" "$work_dir"' EXIT
mkdir "$work_dir/bin" "$work_dir/cargo"
printf 'verified fixture\n' > "$work_dir/fixture"
if command -v sha256sum > /dev/null 2>&1; then
  expected=$(sha256sum "$work_dir/fixture")
else
  expected=$(shasum -a 256 "$work_dir/fixture")
fi
expected=${expected%% *}
verify_sha256 "$expected" "$work_dir/fixture"
printf 'tampered fixture\n' > "$work_dir/fixture"
if verify_sha256 "$expected" "$work_dir/fixture" 2> "$work_dir/hash.log"; then
  echo 'Tampered fixture was accepted' >&2; exit 1
fi

# Simulate a successful HTTP response containing replaced or truncated bytes.
cat > "$work_dir/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -eu
printf 'download\n' >> "$TEST_LOG"
while (( $# )); do
  if [[ $1 == --output ]]; then
    printf 'untrusted replacement bytes\n' > "$2"
    exit 0
  fi
  shift
done
exit 1
EOF
cat > "$work_dir/bin/uname" <<'EOF'
#!/usr/bin/env bash
if [[ $1 == -s ]]; then printf '%s\n' "$TEST_OS"; else printf '%s\n' "$TEST_ARCH"; fi
EOF
for tool in tar unzip; do
  cat > "$work_dir/bin/$tool" <<'EOF'
#!/usr/bin/env bash
printf 'UNSAFE EXTRACTION\n' >> "$TEST_LOG"
exit 1
EOF
done
chmod +x "$work_dir/bin/"*

for host in 'Linux x86_64' 'Linux aarch64' 'Darwin x86_64' 'Darwin arm64' 'MINGW64_NT-10.0 x86_64'; do
  read -r test_os test_arch <<< "$host"
  : > "$work_dir/network.log"
  if PATH="$work_dir/bin:$PATH" CARGO_HOME="$work_dir/cargo" \
      TEST_OS="$test_os" TEST_ARCH="$test_arch" TEST_LOG="$work_dir/network.log" \
      bash "$script_dir/install-cargo-dist.sh" > "$work_dir/attempt.log" 2>&1; then
    echo "Replaced archive was accepted on $host" >&2; exit 1
  fi
  grep -q 'SHA-256 mismatch' "$work_dir/attempt.log"
  [[ $(cat "$work_dir/network.log") == download ]]
  [[ ! -e $work_dir/cargo/bin/dist && ! -e $work_dir/cargo/bin/dist.exe ]]
done

: > "$work_dir/network.log"
if PATH="$work_dir/bin:$PATH" TEST_OS=Unsupported TEST_ARCH=unknown \
    TEST_LOG="$work_dir/network.log" bash "$script_dir/install-cargo-dist.sh" \
    > "$work_dir/attempt.log" 2>&1; then
  echo 'Unsupported host was accepted' >&2; exit 1
fi
[[ ! -s $work_dir/network.log ]]
echo 'cargo-dist download integrity fixtures passed (five hosts and unsupported host).'
