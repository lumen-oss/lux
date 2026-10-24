#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
source_file="$repo_root/xtask/src/static_gpg.rs"
base_url="https://gnupg.org/ftp/gcrypt"

deps=(
    "libgpg-error VERSION SRI 1"
    "libassuan ASSUAN_VERSION ASSUAN_SRI 2"
    "gpgme GPGME_VERSION GPGME_SRI 1"
)

latest_release() {
    local name="$1"
    local major="$2"
    local listing
    listing="$(
        curl -fsSL "$base_url/$name/" \
            | grep -oE "$name-$major\.[0-9]+(\.[0-9]+)?\.tar\.bz2" \
            | sort -uV
    )"
    if [[ -z "$listing" ]]; then
        echo "could not fetch the $name release listing" >&2
        return 1
    fi
    printf '%s\n' "$listing" | sort -V | tail -1
}

current_version() {
    sed -n "s/^const $1: &str = \"\(.*\)\";\$/\1/p" "$source_file"
}

sha256_of() {
    local name="$1"
    local tarball="$2"
    local hash
    hash="sha256-$(
        curl -fsSL "$base_url/$name/$tarball" \
            | openssl dgst -sha256 -binary \
            | openssl base64 -A
    )"
    if [[ -z "$hash" ]]; then
        echo "could not compute the $name checksum" >&2
        return 1
    fi
    printf '%s\n' "$hash"
}

update_source_file() {
    local version_const="$1"
    local sri_const="$2"
    local version="$3"
    local hash="$4"
    local tmp
    tmp="$(mktemp "${source_file}.XXXXXX")"
    sed \
        -e "s|^const $version_const: &str = \".*\";$|const $version_const: \&str = \"$version\";|" \
        -e "s|^const $sri_const: &str = \".*\";$|const $sri_const: \&str = \"$hash\";|" \
        "$source_file" >"$tmp"
    cat "$tmp" >"$source_file"
    rm "$tmp"
}

update_dependency() {
    local spec="$1"
    local name version_const sri_const major tarball version current hash
    read -r name version_const sri_const major <<< "$spec"
    tarball="$(latest_release "$name" "$major")"
    version="${tarball#${name}-}"
    version="${version%.tar.bz2}"
    current="$(current_version "$version_const")"
    if [[ -z "$current" ]]; then
        echo "could not find the $name version pin in $source_file" >&2
        return 1
    fi
    if [[ "$version" == "$current" ]]; then
        echo "$name is up to date ($version)"
        return 0
    fi
    hash="$(sha256_of "$name" "$tarball")"
    update_source_file "$version_const" "$sri_const" "$version" "$hash"
    echo "updated $name: $current -> $version"
}

main() {
    local dep
    for dep in "${deps[@]}"; do
        update_dependency "$dep"
    done
}

main "$@"
