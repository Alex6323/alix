#!/bin/sh
# Run the alix mobile app (apps/mobile) on the named Android AVD, booting that
# emulator first when it is not already up. Resolves the AVD to its adb serial
# by name, so several emulators (phone + tablet) can run side by side. Used by
# `make phone` / `make tablet`.
set -eu

avd="${1:?usage: mobile-run.sh <avd-name>}"
: "${ANDROID_HOME:?ANDROID_HOME is not set (see docs/dev/flutter-android-arch-setup.md)}"
adb="$ANDROID_HOME/platform-tools/adb"

# The adb serial (emulator-55xx) of the running emulator whose AVD name
# matches, if any. `emu avd name` prints the name plus an OK line.
serial_for_avd() {
    for s in $("$adb" devices | awk '/^emulator-/ && $2 == "device" {print $1}'); do
        name=$("$adb" -s "$s" emu avd name 2>/dev/null | head -1 | tr -d '\r')
        if [ "$name" = "$avd" ]; then
            echo "$s"
            return 0
        fi
    done
    return 1
}

if ! serial=$(serial_for_avd); then
    echo "booting emulator $avd ..."
    nohup "$ANDROID_HOME/emulator/emulator" -avd "$avd" >/dev/null 2>&1 &
    i=0
    until serial=$(serial_for_avd); do
        i=$((i + 1))
        if [ "$i" -gt 90 ]; then
            echo "emulator $avd did not come up"
            exit 1
        fi
        sleep 2
    done
    until [ "$("$adb" -s "$serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do
        i=$((i + 1))
        if [ "$i" -gt 150 ]; then
            echo "emulator $avd did not finish booting"
            exit 1
        fi
        sleep 2
    done
fi

echo "running on $avd ($serial)"
cd "$(dirname "$0")/../apps/mobile" && exec flutter run -d "$serial"
