#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}"
mkdir -p "${target_dir}"
lock_dir="${target_dir}/.zip_texts.${PPID}.lock"

if mkdir "${lock_dir}" 2>/dev/null; then
    python3 "${script_dir}/zip_texts.py" "${repo_root}" "${target_dir}/text-files.zip"
fi

rustc="$1"
shift
exec "$rustc" "$@"
