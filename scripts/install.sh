#!/usr/bin/env bash
# Install Dictionary for the current user: the binary, a desktop entry, the
# icon, and (optionally) a palette to follow.
set -euo pipefail
project="$(cd -- "${BASH_SOURCE[0]%/*}/.." && pwd)"
binary="$project/target/release/dictionary"
prefix="$HOME/.local"
config="${XDG_CONFIG_HOME:-$HOME/.config}/dictionary"
apps_list="${XDG_CONFIG_HOME:-$HOME/.config}/apps.list"
follow=""
dry=0
id=io.github.dictionary

usage() {
    printf '%s\n' 'Usage: install.sh [--dry-run] [--binary FILE] [--prefix DIR]' \
        '       [--config-dir DIR] [--apps-list FILE] [--follow PALETTE]' \
        '--apps-list is only written if the file already exists.'
}
fail() { printf 'install: %s\n' "$*" >&2; exit 1; }
safe() { [[ "$2" == /* && "$2" != / && "$2" != *[$'\n|"%\\`$']* ]] || fail "Expected a safe absolute path for $1"; }

while (( $# )); do
    case "$1" in
        --dry-run) dry=1; shift ;;
        -h|--help) usage; exit 0 ;;
        --binary|--prefix|--config-dir|--apps-list|--follow)
            (( $# >= 2 )) || fail "Missing value for $1"
            safe "$1" "$2"
            case "$1" in
                --binary) binary="$2" ;; --prefix) prefix="${2%/}" ;;
                --config-dir) config="${2%/}" ;; --apps-list) apps_list="$2" ;;
                --follow) follow="${2%/}" ;;
            esac
            shift 2 ;;
        *) fail "Unknown option: $1" ;;
    esac
done
[[ -x "$binary" ]] || fail "Build first: cargo build --release ($binary)"

entries=("$prefix/bin/applications" "$prefix/share/applications")
if (( dry )); then
    printf 'Binary:  %s/bin/dictionary\nEntries: %s\nIcon:    %s/share/icons/hicolor/scalable/apps/%s.svg\nApp list: %s\nFollow:  %s\n' \
        "$prefix" "${entries[*]}" "$prefix" "$id" "$apps_list" "${follow:-nothing}"
    exit 0
fi

install -Dm755 -- "$binary" "$prefix/bin/dictionary.new"
mv -T -- "$prefix/bin/dictionary.new" "$prefix/bin/dictionary"
install -Dm644 -- "$project/assets/$id.svg" "$prefix/share/icons/hicolor/scalable/apps/$id.svg"
install -Dm644 -- "$project/data/webster/LICENSE-DATA.txt" "$prefix/share/doc/dictionary/LICENSE-DATA.txt"
for dir in "${entries[@]}"; do
    mkdir -p -- "$dir"
    sed -e "s|@EXEC@|$prefix/bin/dictionary|" -e "s|@CONFIG@|$config|" \
        "$project/packaging/$id.desktop.in" > "$dir/$id.desktop"
    chmod 644 -- "$dir/$id.desktop"
done
command -v desktop-file-validate >/dev/null && desktop-file-validate "$prefix/share/applications/$id.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true

if [[ -e "$apps_list" ]]; then
    target="$(readlink -f -- "$apps_list")"
    if ! awk -F'|' -v id="$id.desktop" '$1==id {found=1} END {exit !found}' "$target"; then
        cp -p -- "$target" "$target.dictionary-backup.$(date +%Y%m%dT%H%M%S)"
        printf '%s.desktop|%s/share/icons/hicolor/scalable/apps/%s.svg|Dictionary\n' "$id" "$prefix" "$id" >> "$target"
    fi
fi
if [[ -n "$follow" ]]; then
    "$prefix/bin/dictionary" --config-dir "$config" theme follow desktop "$follow"
fi
printf 'Installed Dictionary.\n'
