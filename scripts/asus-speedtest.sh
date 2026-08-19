#!/bin/bash
# asus-speedtest.sh — curl-based network speed measurement fallback.
# Output: KEY=VALUE pairs (DOWNLOAD_MBPS, UPLOAD_MBPS, PING_MS, TOOL_MISSING).

# Check if curl is available
if ! command -v curl &>/dev/null; then
    echo "TOOL_MISSING=1"
    exit 0
fi

# ── Latency (ping) ──
PING_MS="N/A"
if command -v ping &>/dev/null; then
    PING_OUT=$(ping -c 3 -W 5 1.1.1.1 2>/dev/null | tail -1)
    if [[ "$PING_OUT" =~ ([0-9]+\.[0-9]+)/([0-9]+\.[0-9]+)/([0-9]+\.[0-9]+) ]]; then
        PING_MS="${BASH_REMATCH[2]}"
    fi
fi

# ── Download speed ──
# Fetch 25 MB from Cloudflare's speed test endpoint and measure throughput.
DL_SPEED=$(curl -o /dev/null -s -w '%{speed_download}' \
    --connect-timeout 10 --max-time 30 \
    'https://speed.cloudflare.com/__down?bytes=25000000' 2>/dev/null)

if [[ -z "$DL_SPEED" || "$DL_SPEED" == "0" || "$DL_SPEED" == "0.000" ]]; then
    DOWNLOAD_MBPS="0.00"
else
    # curl reports bytes/sec, convert to Mbps (megabits)
    DOWNLOAD_MBPS=$(awk "BEGIN { printf \"%.2f\", $DL_SPEED * 8 / 1000000 }")
fi

# ── Upload speed ──
# Generate 5 MB of random data and upload to Cloudflare.
UL_SPEED=$(dd if=/dev/urandom bs=1024 count=5120 2>/dev/null | \
    curl -o /dev/null -s -w '%{speed_upload}' \
    --connect-timeout 10 --max-time 30 \
    -X POST --data-binary @- \
    'https://speed.cloudflare.com/__up' 2>/dev/null)

if [[ -z "$UL_SPEED" || "$UL_SPEED" == "0" || "$UL_SPEED" == "0.000" ]]; then
    UPLOAD_MBPS="0.00"
else
    UPLOAD_MBPS=$(awk "BEGIN { printf \"%.2f\", $UL_SPEED * 8 / 1000000 }")
fi

echo "DOWNLOAD_MBPS=$DOWNLOAD_MBPS"
echo "UPLOAD_MBPS=$UPLOAD_MBPS"
echo "PING_MS=$PING_MS"
