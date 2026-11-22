#!/bin/sh
# Read-only diagnostic; does not alter any graphics or security settings.
set -u
printf '%s\n' '[aster:diagnostic] Linux display and CEF OSR compatibility'
printf 'XDG_SESSION_TYPE=%s\n' "${XDG_SESSION_TYPE:-unset}"
printf 'DISPLAY=%s\n' "${DISPLAY:-unset}"
printf 'WAYLAND_DISPLAY=%s\n' "${WAYLAND_DISPLAY:-unset}"
printf 'XDG_RUNTIME_DIR=%s\n' "${XDG_RUNTIME_DIR:-unset}"
printf 'FONTCONFIG_FILE=%s\n' "${FONTCONFIG_FILE:-unset}"
case "${WAYLAND_DISPLAY:-}" in
  /*) wl_socket="$WAYLAND_DISPLAY" ;;
  '') wl_socket='' ;;
  *) wl_socket="${XDG_RUNTIME_DIR:-}/$WAYLAND_DISPLAY" ;;
esac
if [ -n "$wl_socket" ] && [ -S "$wl_socket" ]; then
    printf 'wayland_socket=present (%s)\n' "$wl_socket"
else
    printf '%s\n' 'wayland_socket=missing_or_not_a_socket'
fi
if command -v nvidia-smi >/dev/null 2>&1; then
    nvidia-smi --query-gpu=name,driver_version,memory.total,memory.used --format=csv,noheader 2>&1 || true
fi
if command -v vulkaninfo >/dev/null 2>&1; then
    vulkaninfo --summary 2>&1 | grep -E 'GPU id|deviceName|driverName|driverInfo|Vulkan Instance' || true
fi
if command -v fc-match >/dev/null 2>&1; then
    fc-match -f 'font_match=%{family}\n' sans 2>&1 || true
fi
for entry in /dev/dri/renderD*; do
    [ -e "$entry" ] || continue
    if [ -r "$entry" ] && [ -w "$entry" ]; then access=rw; else access=restricted; fi
    printf 'render_node=%s access=%s\n' "$entry" "$access"
done
printf '%s\n' '[aster:diagnostic] OOM? in Chromium capture != confirmed VRAM exhaustion'
