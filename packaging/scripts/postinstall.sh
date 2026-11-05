#!/bin/sh
# Creates the service user and lets systemd see the new unit. Nothing is enabled or started.
if command -v systemd-sysusers >/dev/null 2>&1 && [ -f /usr/lib/sysusers.d/rxmltv.conf ]; then
    systemd-sysusers /usr/lib/sysusers.d/rxmltv.conf || true
fi
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
fi
exit 0
