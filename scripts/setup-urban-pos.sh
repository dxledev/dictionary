#!/usr/bin/env bash
set -euo pipefail

data_home="${XDG_DATA_HOME:-${HOME:?HOME must be set}/.local/share}"
prefix="$data_home/dictionary/nltk"
dry_run=0

usage() {
    printf '%s\n' \
        'Usage: setup-urban-pos.sh [--dry-run] [--prefix ABSOLUTE_DIR]' \
        'Creates an isolated Python environment and downloads the English NLTK tagger model.'
}

fail() { printf 'setup-urban-pos: %s\n' "$*" >&2; exit 1; }
validate_path() {
    [[ "$1" == /* && "$1" != / && "$1" != *[$'\n|"%\\`$']* ]] \
        || fail 'Prefix must be a safe absolute path'
}

while (( $# )); do
    case "$1" in
        --dry-run) dry_run=1; shift ;;
        -h|--help) usage; exit 0 ;;
        --prefix)
            (( $# >= 2 )) || fail 'Missing value for --prefix'
            validate_path "$2"
            prefix="${2%/}"
            shift 2 ;;
        *) fail "Unknown option: $1" ;;
    esac
done

validate_path "$prefix"
python="${prefix%/}/bin/python"
nltk_data="${prefix%/}/nltk_data"

if (( dry_run )); then
    printf 'Virtual environment: %s\nNLTK data:          %s\n' "$prefix" "$nltk_data"
    printf 'Commands: python3 -m venv %q; %q -m pip install "nltk==3.10.4" "numpy>=2,<3"; NLTK_DATA=%q %q -m nltk.downloader -d %q averaged_perceptron_tagger_eng\n' \
        "$prefix" "$python" "$nltk_data" "$python" "$nltk_data"
    exit 0
fi

command -v python3 >/dev/null 2>&1 || fail 'python3 is required'
python3 -m venv "$prefix"
"$python" -m pip install 'nltk==3.10.4' 'numpy>=2,<3'
mkdir -p -- "$nltk_data"
NLTK_DATA="$nltk_data" "$python" -m nltk.downloader -d "$nltk_data" averaged_perceptron_tagger_eng
printf 'Installed Urban POS fallback in %s\n' "$prefix"
