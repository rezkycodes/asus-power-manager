#!/usr/bin/env bash
# Print a DIMM/memory hardware summary parsed from dmidecode.
# Requires root (invoked via sudo through the sudoers libexec wildcard).
# Output: KEY=VALUE lines consumed by the Rust GUI Memory tab.
set -euo pipefail

OUT="$(dmidecode -t memory 2>/dev/null || true)"

# Memory type (DDR4/DDR5/...) and physical form factor of the first module.
TYPE=$(printf '%s\n' "$OUT" | awk -F': ' '/^\tType:/ {print $2; exit}')
FORM=$(printf '%s\n' "$OUT" | awk -F': ' '/^\tForm Factor:/ {print $2; exit}')

# Prefer the configured (running) speed; fall back to the rated speed.
SPEED=$(printf '%s\n' "$OUT" | awk -F': ' '/^\tConfigured Memory Speed:/ {if ($2 != "Unknown") {print $2; exit}}')
[ -z "${SPEED:-}" ] && SPEED=$(printf '%s\n' "$OUT" | awk -F': ' '/^\tSpeed:/ {if ($2 != "Unknown") {print $2; exit}}')

# Slot occupancy: populated modules (Size has a number) out of total device handles.
TOTAL=$(printf '%s\n' "$OUT" | grep -c '^Memory Device$' || true)
USED=$(printf '%s\n' "$OUT" | awk -F': ' '/^\tSize:/ {if ($2 ~ /[0-9]/) c++} END {print c+0}')

echo "TYPE=${TYPE:-Unknown}"
echo "FORM=${FORM:-Unknown}"
echo "SPEED=${SPEED:-Unknown}"
echo "SLOTS=${USED:-0}/${TOTAL:-0}"
