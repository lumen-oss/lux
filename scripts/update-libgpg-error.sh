#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
source_file="$repo_root/xtask/src/static_libgpg_error.rs"
base_url="https://gnupg.org/ftp/gcrypt/libgpg-error"

latest_release() {
    local listing
    listing="$(curl -fsSL "$base_url/" | grep -oE 'libgpg-error-[0-9]+\.[0-9]+(\.[0-9]+)?\.tar\.bz2' | sort -uV)"
    if [[ -z "$listing" ]]; then
        echo "could not fetch the libgpg-error release listing" >&2
        return 1
    fi
    printf '%s\n' "$listing" | sort -V | tail -1
}

current_version() {
    sed -n 's/^const VERSION: &str = "\(.*\)";$/\1/p' "$source_file"
}

sha256_of() {
    local tarball="$1"
    local hash
    hash="sha256-$(curl -fsSL "$base_url/$tarball" | openssl dgst -sha256 -binary | openssl base64 -A)"
    if [[ -z "$hash" ]]; then
        echo "could not compute the libgpg-error checksum" >&2
        return 1
    fi
    printf '%s\n' "$hash"
}

update_source_file() {
    local version="$1"
    local hash="$2"
    local tmp
    tmp="$(mktemp "${source_file}.XXXXXX")"
    sed \
        -e "s|^const VERSION: &str = \".*\";$|const VERSION: \&str = \"$version\";|" \
        -e "s|^const SHA256: &str = \".*\";$|const SHA256: \&str = \"$hash\";|" \
        "$source_file" >"$tmp"
    cat "$tmp" >"$source_file"
    rm "$tmp"
}

main() {
    local tarball version current hash
    tarball="$(latest_release)"
    version="${tarball#libgpg-error-}"
    version="${version%.tar.bz2}"
    current="$(current_version)"
    if [[ -z "$current" ]]; then
        echo "could not find the libgpg-error version pin in $source_file" >&2
        return 1
    fi
    if [[ "$version" == "$current" ]]; then
        echo "libgpg-error is up to date ($version)"
        return 0
    fi
    hash="$(sha256_of "$tarball")"
    update_source_file "$version" "$hash"
    echo "updated libgpg-error: $current -> $version"
}

main "$@"
