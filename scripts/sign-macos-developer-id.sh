#!/usr/bin/env bash
# Dormant Developer ID path.
#
# v1 ships ad-hoc sealed builds. When APPLE_CERTIFICATE (and the rest of the
# APPLE_* secrets) exist, the release workflow calls this script instead of
# the ad-hoc reseal so signing can be enabled without restructuring the job.
# The script fails on purpose until that work is implemented. Do not set the
# secrets until this script imports the certificate, signs with the Developer
# ID identity, and notarizes.
set -euo pipefail

echo "Developer ID signing is not implemented." >&2
echo "Unset APPLE_CERTIFICATE to publish the ad-hoc sealed DMG." >&2
exit 1
