#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
tiniest_rs="$repo_root/lux-lib/src/operations/test/tiniest.rs"
api="https://api.github.com/repos/dphfox/tiniest/commits/main"

curl_args=(-fsSL)
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $GITHUB_TOKEN")
fi

latest="$(curl "${curl_args[@]}" "$api" | jq -r '.sha')"
current_sha="$(sed -n 's/.*tag = "\([0-9a-f]\{40\}\)".*/\1/p' "$tiniest_rs")"
current_version="$(sed -n 's/^const TINIEST_VERSION: &str = "\(.*\)";$/\1/p' "$tiniest_rs")"

if [[ -z "$latest" || "$latest" == "null" ]]; then
    echo "could not determine the latest tiniest commit" >&2
    exit 1
fi

if [[ -z "$current_sha" || -z "$current_version" ]]; then
    echo "could not find the tiniest pin in $tiniest_rs" >&2
    exit 1
fi

if [[ "$latest" == "$current_sha" ]]; then
    echo "tiniest is up to date ($current_sha)"
    exit 0
fi

new_version="$(awk -F'[-.]' '{printf "%d.%d.%d-%d", $1, $2, $3 + 1, $4}' <<<"$current_version")"

tmp="$(mktemp "${tiniest_rs}.XXXXXX")"
sed \
    -e "s|tag = \"[0-9a-f]\{40\}\"|tag = \"$latest\"|" \
    -e "s|^const TINIEST_VERSION: &str = \".*\";$|const TINIEST_VERSION: \&str = \"$new_version\";|" \
    "$tiniest_rs" >"$tmp"
cat "$tmp" >"$tiniest_rs"
rm "$tmp"

echo "updated tiniest: $current_sha -> $latest ($current_version -> $new_version)"
