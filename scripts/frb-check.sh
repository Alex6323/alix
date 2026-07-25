#!/bin/sh
# Assert the frb (flutter_rust_bridge) toolchain-alignment invariants for
# apps/mobile and FAIL on drift, so a broken mobile build names its cause
# instead of surfacing as a Gradle stacktrace. Run via `make frb-check`;
# the mobile CI runs it before building. See docs/dev/frb-bridge-setup.md
# for what each invariant is and how to fix a drifted one.
#
# Checked (FAIL): codegen binary == pubspec pin == rust crate pin (exact);
# the two template patches (cargokit ExecOperations, rust_builder
# compileSdk >= 36); the app's exact NDK is installed (that is the NDK cargokit
# builds the Rust with, via Gradle's android.ndkVersion).
# Checked (warn): ANDROID_NDK_HOME pointing at a different NDK than the
# build uses. That only affects manual `cargo ndk` runs, not the app build.
set -eu

app="apps/mobile"
fail=0

ok()   { printf 'ok:   %s\n' "$1"; }
bad()  { printf 'FAIL: %s\n' "$1"; fail=1; }
warn() { printf 'warn: %s\n' "$1"; }

# 1. The codegen binary version.
if command -v flutter_rust_bridge_codegen >/dev/null 2>&1; then
    codegen=$(flutter_rust_bridge_codegen --version | awk '{print $NF}')
else
    bad "flutter_rust_bridge_codegen is not installed (cargo install flutter_rust_bridge_codegen)"
    codegen=""
fi

# 2. The Dart package pin: exact (a ^ or ~ range can drift past the codegen).
dart_pin=$(sed -n 's/^  flutter_rust_bridge: \([0-9][0-9.]*\)$/\1/p' "$app/pubspec.yaml")
if [ -z "$dart_pin" ]; then
    bad "pubspec.yaml: flutter_rust_bridge is not pinned to an exact version"
else
    ok "pubspec.yaml pins flutter_rust_bridge $dart_pin (exact)"
fi

# 3. The Rust crate pin: exact ("=X.Y.Z").
rust_pin=$(sed -n 's/^flutter_rust_bridge = "=\([0-9][0-9.]*\)"$/\1/p' "$app/rust/Cargo.toml")
if [ -z "$rust_pin" ]; then
    bad "rust/Cargo.toml: flutter_rust_bridge is not pinned exactly (\"=X.Y.Z\")"
else
    ok "rust/Cargo.toml pins flutter_rust_bridge =$rust_pin"
fi

# 4. All three versions agree.
if [ -n "$codegen" ] && [ -n "$dart_pin" ] && [ -n "$rust_pin" ]; then
    if [ "$codegen" = "$dart_pin" ] && [ "$dart_pin" = "$rust_pin" ]; then
        ok "codegen $codegen == Dart pin == Rust pin"
    else
        bad "version skew: codegen $codegen, Dart pin $dart_pin, Rust pin $rust_pin (must all match)"
    fi
fi

# 5. The cargokit Gradle-9 patch: ExecOperations in, Project.exec() out.
plugin="$app/rust_builder/cargokit/gradle/plugin.gradle"
if grep -q "ExecOperations" "$plugin" && ! grep -q "project\.exec" "$plugin"; then
    ok "cargokit plugin.gradle carries the ExecOperations patch"
else
    bad "cargokit plugin.gradle lost the ExecOperations patch (Gradle 9 removed Project.exec())"
fi

# 6. The rust_builder compileSdk patch (33 is too old for current androidx).
csdk=$(sed -n 's/^ *compileSdkVersion \([0-9][0-9]*\)$/\1/p' "$app/rust_builder/android/build.gradle")
if [ -n "$csdk" ] && [ "$csdk" -ge 36 ]; then
    ok "rust_builder compileSdkVersion $csdk (>= 36)"
else
    bad "rust_builder compileSdkVersion is '$csdk' (must be >= 36)"
fi

# 7. The app's exact NDK is installed. Scheduled mobile drift temporarily
# rewrites this pin in its checkout to the current Flutter NDK before calling
# this check; normal CI and release builds never derive it from a moving SDK.
gradle="$app/android/app/build.gradle.kts"
app_ndk=$(sed -n 's/^[[:space:]]*ndkVersion = "\([0-9][0-9.]*\)"$/\1/p' "$gradle")
if [ -z "$app_ndk" ]; then
    bad "could not read an exact ndkVersion from $gradle"
else
    if [ -n "${ANDROID_HOME:-}" ] && [ -d "$ANDROID_HOME/ndk/$app_ndk" ]; then
        ok "app-pinned NDK $app_ndk is installed"
    else
        bad "app pins NDK $app_ndk but it is not under \$ANDROID_HOME/ndk (sdkmanager \"ndk;$app_ndk\")"
    fi
    # ANDROID_NDK_HOME only steers manual `cargo ndk` runs; a mismatch does
    # not break the app build but makes a manual repro use a different NDK.
    if [ -n "${ANDROID_NDK_HOME:-}" ]; then
        if [ "$(basename "$ANDROID_NDK_HOME")" != "$app_ndk" ]; then
            warn "ANDROID_NDK_HOME is $(basename "$ANDROID_NDK_HOME"), the build uses $app_ndk (manual cargo-ndk runs differ)"
        else
            ok "ANDROID_NDK_HOME matches the build NDK"
        fi
    else
        warn "ANDROID_NDK_HOME unset in this shell; not compared (only manual cargo-ndk runs read it)"
    fi
fi

if [ "$fail" -ne 0 ]; then
    echo "frb-check: DRIFT DETECTED (see FAIL lines above)"
    exit 1
fi
echo "frb-check: all invariants hold"
