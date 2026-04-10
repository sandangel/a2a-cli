#!/usr/bin/env bash
# Show an art file — clears screen then cats it.
# Usage: scripts/show-art.sh art/intro.txt
#        scripts/show-art.sh intro          (looks up art/<name>.txt)
set -euo pipefail

arg="${1:-intro}"

# Accept either a full path (art/intro.txt) or just a name (intro)
if [ -f "$arg" ]; then
  file="$arg"
else
  file="art/${arg}.txt"
fi

[ -f "$file" ] || { echo "Art file not found: $file"; exit 1; }
clear
cat "$file"
