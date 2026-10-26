#!/usr/bin/env bash
# Install the Changie release used by CI and tag-release.
set -euo pipefail

version="1.26.0"
asset="changie_${version}_linux_amd64.tar.gz"
url="https://github.com/miniscruff/changie/releases/download/v${version}/${asset}"
curl -fsSL -o /tmp/changie.tar.gz "${url}"
sudo tar -xzf /tmp/changie.tar.gz -C /usr/local/bin changie
changie --version
