#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
build_lua="$repo_root/lux-lib/src/operations/build_lua.rs"
base_url="https://www.lua.org/ftp"

listing="$(curl -fsSL "$base_url/" | grep -oE 'lua-[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz' | sort -uV)"

if [[ -z "$listing" ]]; then
    echo "could not fetch the Lua release listing" >&2
    exit 1
fi

codes="$(grep -oE '^const LUA[0-9]{2}_VERSION' "$build_lua" | grep -oE '[0-9]{2}')"

if [[ -z "$codes" ]]; then
    echo "could not find Lua version pins in $build_lua" >&2
    exit 1
fi

for code in $codes; do
    minor="${code:0:1}.${code:1:1}"
    minor_re="${minor//./\\.}"
    tarball="$(printf '%s\n' "$listing" | grep -E "^lua-${minor_re}\.[0-9]+\.tar\.gz$" | sort -V | tail -1)"
    version="${tarball#lua-}"
    version="${version%.tar.gz}"

    current_version="$(sed -n "s/^const LUA${code}_VERSION: &str = \"\(.*\)\";$/\1/p" "$build_lua")"

    if [[ -z "$version" || -z "$current_version" ]]; then
        echo "could not determine the current or latest lua $minor version" >&2
        continue
    fi

    if [[ "$version" == "$current_version" ]]; then
        echo "lua $minor is up to date ($version)"
        continue
    fi

    hash="sha256-$(curl -fsSL "$base_url/$tarball" | openssl dgst -sha256 -binary | openssl base64 -A)"

    tmp="$(mktemp "${build_lua}.XXXXXX")"
    sed \
        -e "s|^const LUA${code}_VERSION: &str = \".*\";$|const LUA${code}_VERSION: \&str = \"$version\";|" \
        -e "s|^const LUA${code}_HASH: &str = \".*\";$|const LUA${code}_HASH: \&str = \"$hash\";|" \
        "$build_lua" >"$tmp"
    cat "$tmp" >"$build_lua"
    rm "$tmp"

    echo "updated lua $minor: $current_version -> $version"
done
