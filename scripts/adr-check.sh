#!/bin/sh
# An Accepted ADR asserts a constraint that is IN FORCE. This proves that every
# marker such a record names is actually present in the tree, so a status cannot
# drift ahead of the code. ADR 0028 read as shipped for months while none of it
# existed; a costed docs audit caught it, not the inner loop.
#
# A record names its marker with one or more lines:
#     - Evidence: <literal string> in <path>
# The string must occur under that path. Use `- Evidence: none, <why>` for a
# record that constrains no identifier, such as a scope boundary or a policy.
#
# Records carrying no Evidence line are counted and listed, not failed: a marker
# invented in a hurry gives false assurance, which is worse than none.
set -eu

dir="docs/adrs"
status_file="${TMPDIR:-/tmp}/adr-check.$$"
failed=0
missing=""

note() { echo "adr-check: $1" >&2; }
trap 'rm -f "$status_file"' EXIT

for adr in "$dir"/0*.md; do
    status=$(sed -n 's/^- Status: *//p' "$adr" | head -n 1)
    case "$status" in
        Accepted*) ;;
        *) continue ;;
    esac
    # A record whose own status says it is unimplemented is honest, not drift.
    case "$status" in
        *"NOT yet implemented"*) continue ;;
    esac

    sed -n 's/^- Evidence: *//p' "$adr" > "$status_file"
    if [ ! -s "$status_file" ]; then
        missing="$missing $(basename "$adr")"
        continue
    fi

    while IFS= read -r line; do
        case "$line" in
            none*) continue ;;
            *" in "*) ;;
            *)
                note "$(basename "$adr"): malformed evidence '$line'"
                note "  expected '- Evidence: <string> in <path>'"
                failed=1
                continue
                ;;
        esac
        needle=${line%% in *}
        path=${line##* in }
        if [ ! -e "$path" ]; then
            note "$(basename "$adr"): evidence path '$path' does not exist"
            failed=1
        elif ! grep -rqF -- "$needle" "$path"; then
            note "$(basename "$adr"): Accepted, but '$needle' is absent from $path"
            note "  either the code regressed, or the status is ahead of it"
            failed=1
        fi
    done < "$status_file"
done

[ "$failed" -eq 0 ] || exit 1

if [ -n "$missing" ]; then
    count=$(echo "$missing" | wc -w | tr -d ' ')
    note "$count Accepted record(s) name no evidence yet:$missing"
fi

echo "adr-check: every named marker is present"
