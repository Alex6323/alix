#!/bin/sh
# Keep release/CI toolchains reproducible while allowing named drift workflows
# to exercise current upstream toolchains deliberately.
set -eu

fail=0

ok()  { printf 'ok:   %s\n' "$1"; }
bad() { printf 'FAIL: %s\n' "$1" >&2; fail=1; }

require_literal() {
    file=$1
    literal=$2
    description=$3
    if grep -Fq "$literal" "$file"; then
        ok "$description"
    else
        bad "$description ($file must contain: $literal)"
    fi
}

rust=$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' rust-toolchain.toml)
case "$rust" in
    [0-9]*.[0-9]*.[0-9]*) ok "Rust toolchain is exact ($rust)" ;;
    *) bad "rust-toolchain.toml must pin an exact X.Y.Z channel (found '$rust')" ;;
esac

nightly=$(cat .rust-nightly-version)
case "$nightly" in
    nightly-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9])
        ok "nightly toolchain is date-pinned ($nightly)"
        ;;
    *) bad ".rust-nightly-version must contain nightly-YYYY-MM-DD (found '$nightly')" ;;
esac

flutter=$(sed -n 's/^[[:space:]]*"flutter":[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' apps/mobile/.fvmrc)
case "$flutter" in
    [0-9]*.[0-9]*.[0-9]*) ok "Flutter toolchain is exact ($flutter)" ;;
    *) bad "apps/mobile/.fvmrc must pin an exact X.Y.Z Flutter version (found '$flutter')" ;;
esac

ndk=$(sed -n 's/^[[:space:]]*ndkVersion = "\([0-9][0-9.]*\)"$/\1/p' apps/mobile/android/app/build.gradle.kts)
case "$ndk" in
    [0-9]*.[0-9]*.[0-9]*) ok "Android NDK is exact ($ndk)" ;;
    *) bad "apps/mobile/android/app/build.gradle.kts must pin an exact NDK (found '$ndk')" ;;
esac

frb=$(sed -n 's/^  flutter_rust_bridge: \([0-9][0-9.]*\)$/\1/p' apps/mobile/pubspec.yaml)
case "$frb" in
    [0-9]*.[0-9]*.[0-9]*) ok "flutter_rust_bridge codegen is exact ($frb)" ;;
    *) bad "apps/mobile/pubspec.yaml must pin an exact flutter_rust_bridge version (found '$frb')" ;;
esac
frb_rust=$(sed -n 's/^flutter_rust_bridge = "=\([0-9][0-9.]*\)"$/\1/p' apps/mobile/rust/Cargo.toml)
if [ "$frb" = "$frb_rust" ]; then
    ok "Dart and Rust flutter_rust_bridge pins agree"
else
    bad "flutter_rust_bridge mismatch: Dart $frb, Rust $frb_rust"
fi

for workflow in .github/workflows/*.yml; do
    while IFS= read -r value; do
        value=${value%%#*}
        value=$(printf '%s' "$value" | tr -d '[:space:]')
        case "$value" in
            ./*) ;;
            *@*)
                ref=${value##*@}
                if ! printf '%s\n' "$ref" | grep -Eq '^[0-9a-f]{40}$'; then
                    bad "$workflow has a movable action reference: $value"
                fi
                ;;
            *) bad "$workflow has an invalid action reference: $value" ;;
        esac
    done <<EOF
$(sed -n 's/^[[:space:]-]*uses:[[:space:]]*//p' "$workflow")
EOF
done
if [ "$fail" -eq 0 ]; then
    ok "every external GitHub Action uses a full commit SHA"
fi

# Enumerated from the files, never from a list of known workflows: a pin added
# to a new workflow, or a stale one beside a correct one, must not pass unseen.
stray=$(
    for workflow in .github/workflows/*.yml; do
        case "$workflow" in
            .github/workflows/backend-drift.yml) continue ;;
            .github/workflows/mobile-drift.yml) continue ;;
        esac
        awk -v file="$workflow" -v rust="$rust" -v nightly="$nightly" '
            /^[[:space:]]*toolchain:[[:space:]]*/ {
                value = $2
                gsub(/"/, "", value)
                if (value != rust && value != nightly) {
                    printf "  %s:%d has toolchain: %s\n", file, NR, value
                }
            }' "$workflow"
    done
)
if [ -n "$stray" ]; then
    bad "every workflow Rust pin must be $rust or $nightly outside named drift jobs"
    printf '%s\n' "$stray" >&2
else
    ok "every workflow Rust pin is exact ($rust / $nightly)"
fi

require_literal Makefile 'RUST_NIGHTLY := $(shell cat .rust-nightly-version)' \
    "Make targets read the date-pinned nightly"
require_literal Makefile \
    'RUST_TOOLCHAIN := $(shell sed -n '\''s/^channel = "\([^"]*\)"$$/\1/p'\'' rust-toolchain.toml)' \
    "Make targets read the exact Rust toolchain"
require_literal Makefile 'cargo +$(RUST_TOOLCHAIN) install --locked --path .' \
    "local install selects the exact Rust toolchain and lockfile"
require_literal .github/workflows/ci.yml "toolchain: $rust" \
    "CI uses the exact Rust toolchain"
require_literal .github/workflows/release.yml "toolchain: $rust" \
    "desktop releases use the exact Rust toolchain"
require_literal .github/workflows/ci.yml "toolchain: $nightly" \
    "formatting and coverage use the date-pinned nightly"
require_literal .github/workflows/ci.yml "flutter-version-file: apps/mobile/.fvmrc" \
    "mobile CI reads the exact Flutter pin"
require_literal .github/workflows/mobile-release.yml "flutter-version-file: apps/mobile/.fvmrc" \
    "mobile releases read the exact Flutter pin"
require_literal .github/workflows/mobile-release.yml 'java-version: "21.0.11+10"' \
    "mobile releases pin the Temurin patch"
require_literal .github/workflows/ci.yml 'node-version: "20.18.0"' \
    "blocking JavaScript jobs pin Node"
require_literal .github/workflows/pages.yml 'tool: mdbook@0.5.3' \
    "Pages pins mdBook"
require_literal .github/workflows/ci.yml 'tool: cargo-llvm-cov@0.8.5' \
    "coverage pins cargo-llvm-cov"
require_literal .github/workflows/mobile-release.yml \
    'gradle="apps/mobile/android/app/build.gradle.kts"' \
    "mobile releases read the NDK from the app pin"
require_literal .github/workflows/mobile-release.yml \
    "cargo install --locked --version $frb flutter_rust_bridge_codegen" \
    "mobile releases install exact locked FRB codegen"
require_literal .github/workflows/mobile-drift.yml \
    "cargo install --locked --version $frb flutter_rust_bridge_codegen" \
    "mobile drift keeps FRB codegen aligned"
require_literal .github/workflows/mobile-drift.yml "channel: stable" \
    "mobile drift deliberately follows stable Flutter"
require_literal .github/workflows/mobile-drift.yml "toolchain: stable" \
    "mobile drift deliberately follows stable Rust"
require_literal .github/workflows/backend-drift.yml "toolchain: stable" \
    "backend drift deliberately follows stable Rust"
require_literal .github/dependabot.yml 'package-ecosystem: "github-actions"' \
    "Dependabot maintains Action SHA pins"

if grep -R -n 'cargo-bins/cargo-binstall@' .github/workflows >/dev/null; then
    bad "workflows must not execute the mutable cargo-binstall action"
else
    ok "no workflow executes cargo-binstall as an Action"
fi

if [ "$fail" -ne 0 ]; then
    echo "toolchain-check: DRIFT DETECTED" >&2
    exit 1
fi
echo "toolchain-check: all invariants hold"
