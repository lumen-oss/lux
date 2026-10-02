#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
build_lua="$repo_root/lux-lib/src/operations/build_lua.rs"
api="https://api.github.com/repos/luau-lang/luau/releases/latest"

curl_args=(-fsSL)
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $GITHUB_TOKEN")
fi

latest="$(curl "${curl_args[@]}" "$api" | jq -r '.tag_name')"
current="$(sed -n 's/^const LUAU_VERSION: &str = "\(.*\)";$/\1/p' "$build_lua")"

if [[ -z "$latest" || "$latest" == "null" ]]; then
    echo "could not determine the latest Luau release" >&2
    exit 1
fi

if [[ -z "$current" ]]; then
    echo "could not find LUAU_VERSION in $build_lua" >&2
    exit 1
fi

if [[ "$latest" == "$current" ]]; then
    echo "luau is up to date ($current)"
    exit 0
fi

tmp="$(mktemp "${build_lua}.XXXXXX")"
sed "s|^const LUAU_VERSION: &str = \".*\";$|const LUAU_VERSION: \&str = \"$latest\";|" "$build_lua" >"$tmp"
cat "$tmp" >"$build_lua"
rm "$tmp"

echo "updated LUAU_VERSION: $current -> $latest"
