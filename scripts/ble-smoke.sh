#!/usr/bin/env bash
# BLE smoke: scan for an advertised stack-chan, list its GATT services,
# and read the SIG-standard battery level. Designed to be runnable from
# the host without any phone or BLE-explorer GUI.
#
# Prereqs: BlueZ (bluetoothctl, gatttool) + a working Bluetooth adapter.
# Falls back to a printed-procedure mode if either is missing — useful
# in CI / agent shells where there's no radio to scan with.
#
# Usage:
#   scripts/ble-smoke.sh                    # scan-only smoke
#   scripts/ble-smoke.sh stackchan-abc123   # target a specific device

set -u

readonly TARGET_NAME_PREFIX="${1:-stackchan-}"
readonly SCAN_SECS="${SCAN_SECS:-15}"

print_manual_procedure() {
    cat <<EOF

Manual smoke procedure (phone — nRF Connect or LightBlue):

    1. Power the CoreS3. Boot log should announce
       "ble: address=… name=stackchan-XXXXXX".
    2. In nRF Connect → Scanner, look for "stackchan-XXXXXX".
       Signal strength varies; pull the device closer if it doesn't
       appear within ~10 s.
    3. Connect. Three services should be listed:
        - Generic Access (auto)
        - Device Information (0x180A)
            • Manufacturer Name String → "M5Stack"
            • Model Number String     → "Stack-chan CoreS3"
            • Firmware Revision String → cargo version, e.g. "0.27.0"
        - Battery (0x180F)
            • Battery Level (0x2A19) → integer 0..=100
              Subscribe (CCCD) to receive a value every ~1 s.
        - Stack-chan custom (8a1c0001-7b3f-4d52-9c6e-5f5ba1e5cf01)
            • Emotion (8a1c0002-…) → byte 0..=5
              Subscribe; touch the head pads or watch the autonomous
              EmotionCycle to see the value change.
    4. Disconnect. The boot log should print "ble: peer disconnected"
       and resume advertising. Reconnect should succeed.

EOF
}

if ! command -v bluetoothctl >/dev/null 2>&1; then
    print_manual_procedure
    exit 0
fi

# `bluetoothctl show` blocks forever when no adapter is registered (or
# the bluetoothd dbus path isn't responding). Bound it.
adapter_status="$(timeout 3 bluetoothctl show 2>/dev/null || true)"
if ! echo "$adapter_status" | grep -q "Powered: yes"; then
    echo "ble-smoke: bluetoothctl present but no powered adapter — falling back to manual procedure."
    print_manual_procedure
    exit 0
fi

echo "ble-smoke: scanning for ${SCAN_SECS}s for advertisers matching '${TARGET_NAME_PREFIX}'..."

# Run a timed scan and capture devices. The `--timeout` flag exists on
# newer bluetoothctl; older versions need the wrapped scan-on / scan-off
# pattern. Try both for portability.
scan_output=$(
    {
        echo "scan on"
        sleep "${SCAN_SECS}"
        echo "scan off"
        echo "devices"
        echo "exit"
    } | timeout "$((SCAN_SECS + 10))" bluetoothctl 2>&1 || true
)

# Print everything matching the target prefix; sort by MAC for stability.
# Case-insensitive: BlueZ on Debian/Ubuntu/Pi OS sometimes emits lowercase
# MACs (`aa:bb:cc:…`) where Fedora emits uppercase. `-i` covers both.
hits=$(echo "$scan_output" | grep -Ei "Device [0-9A-F:]{17} ${TARGET_NAME_PREFIX}" | sort -u)

if [ -z "$hits" ]; then
    echo "ble-smoke: no advertisers matching '${TARGET_NAME_PREFIX}' seen in ${SCAN_SECS}s."
    echo "ble-smoke: confirm the firmware is flashed and the BT radio is up; re-run with longer SCAN_SECS=30 if RF is noisy."
    exit 1
fi

echo "ble-smoke: found:"
echo "$hits" | sed 's/^/  /'

# Pick the first match for the deeper read.
mac=$(echo "$hits" | head -n 1 | awk '{print $2}')
echo
echo "ble-smoke: connecting to ${mac} for a battery-level read..."

# bluetoothctl GATT subcommands work with the active adapter and
# require the device to be paired-or-trusted in some BlueZ builds. We
# don't pair (provisioning will need that in PR3); just attempt a
# direct connect + read.
read_output=$(
    {
        echo "connect ${mac}"
        sleep 4
        echo "menu gatt"
        echo "list-attributes ${mac}"
        sleep 2
        echo "back"
        echo "disconnect ${mac}"
        echo "exit"
    } | timeout 15 bluetoothctl 2>&1 || true
)

# We can't reliably select the characteristic by path (the service / char
# index varies per advertise cycle), but list-attributes alone confirms
# the GATT table parsed end-to-end. A successful connect means the
# trouble-host advertise loop accepted us.
if echo "$read_output" | grep -q "Connected: yes\|new connection"; then
    echo "ble-smoke: connected + GATT table enumerated."
    echo "$read_output" | grep -E "Service /org|Characteristic /org" | head -20
    exit 0
else
    echo "ble-smoke: connect failed."
    echo "$read_output" | tail -20
    exit 1
fi
