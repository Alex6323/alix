#!/bin/sh
# One-way, dev-only: copy a host decks folder into the alix mobile app's
# private decks dir on the running emulator (the debug build allows run-as).
# The app lists it after a restart. Never syncs anything back; progress made
# on the emulator stays there. Usage: `make push-decks DIR=~/decks`.
set -eu

dir="${1:?usage: push-decks.sh <decks-dir>}"
: "${ANDROID_HOME:?ANDROID_HOME is not set (see docs/dev/flutter-android-arch-setup.md)}"
adb="$ANDROID_HOME/platform-tools/adb"

serial=$("$adb" devices | awk '/^emulator-/ && $2 == "device" {print $1; exit}')
if [ -z "$serial" ]; then
    echo "no running emulator (start one with make phone or make tablet)"
    exit 1
fi

# adb shell flattens its arguments into one remote string, so the whole
# run-as command travels as a single quoted argument or the remote sh
# word-splits it apart.
"$adb" -s "$serial" push "$dir" /data/local/tmp/alix-push > /dev/null
"$adb" -s "$serial" shell \
    "run-as study.alix.mobile sh -c 'mkdir -p files/decks && cp -r /data/local/tmp/alix-push/. files/decks/'"
"$adb" -s "$serial" shell rm -rf /data/local/tmp/alix-push
echo "pushed $dir into files/decks/ on $serial (restart the app to re-list)"
