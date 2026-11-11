#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
work_dir=$(mktemp -d)
trap 'rm -f "$work_dir/bin/cargo" "$work_dir/tool/bin/cargo-audit" "$work_dir/install.log"; rmdir "$work_dir/bin" "$work_dir/tool/bin" "$work_dir/tool" "$work_dir"' EXIT
mkdir "$work_dir/bin"
cat > "$work_dir/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ $1 == install && $2 == --locked && $3 == --version && $4 == =0.23.0 && $5 == --root && $7 == cargo-audit ]]
printf 'install\n' >> "$TEST_INSTALL_LOG"
mkdir -p "$6/bin"
printf '#!/usr/bin/env bash\nprintf "cargo-audit 0.23.0\\n"\n' > "$6/bin/cargo-audit"
chmod +x "$6/bin/cargo-audit"
EOF
chmod +x "$work_dir/bin/cargo"
export TEST_INSTALL_LOG="$work_dir/install.log"
PATH="$work_dir/bin:$PATH" bash "$script_dir/ensure-cargo-audit.sh" 0.23.0 "$work_dir/tool" false
PATH="$work_dir/bin:$PATH" bash "$script_dir/ensure-cargo-audit.sh" 0.23.0 "$work_dir/tool" true
if PATH="$work_dir/bin:$PATH" bash "$script_dir/ensure-cargo-audit.sh" 0.23.0 "$work_dir/tool" false; then
  echo 'Non-exact cached executable was accepted' >&2; exit 1
fi
[[ $(wc -l < "$work_dir/install.log" | tr -d ' ') == 1 ]]
printf '#!/usr/bin/env bash\nprintf "cargo-audit 0.22.2\\n"\n' > "$work_dir/tool/bin/cargo-audit"
if PATH="$work_dir/bin:$PATH" bash "$script_dir/ensure-cargo-audit.sh" 0.23.0 "$work_dir/tool" true; then
  echo 'Unexpected cached version was accepted' >&2; exit 1
fi
[[ $(wc -l < "$work_dir/install.log" | tr -d ' ') == 1 ]]
echo 'cargo-audit cache fixtures passed (cold install, warm reuse, wrong-version rejection).'
