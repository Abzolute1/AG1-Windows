#!/usr/bin/env bash
set -euo pipefail
# Authenticate locally in your terminal; no password is stored in this script.
sudo python3 /home/alex/Projects/AG1/product/release/install-systemwide.py \
  /home/alex/Projects/AG1/.local/p14-packages/97e8685bcd5e4dc9a4e7a0605e69cae0/ag1-0.1.0-rc-linux-x86_64.tar.gz \
  95ec6125faaec2a317267ad593e986044d6f2b9653d11bbb8275c364e0998a21
