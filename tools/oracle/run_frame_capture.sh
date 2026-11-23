#!/usr/bin/env bash
# Shell-native per-frame Ikemen-GO oracle capture.
#
# Some desktop/offscreen GL setups only launch reliably from a shell with a
# session, not from a Python child process. This wrapper prepares the runtime,
# launches the game, and hands the capture to the Python reducer.
#
# Usage:
#   IKEMEN_DIR=/path/to/ikemen-runtime tools/oracle/run_frame_capture.sh
#   IKEMEN_ORACLE_SCENARIO=kfm_attack IKEMEN_ORACLE_KEEP=1 ... 
set -euo pipefail

root="${IKEMEN_DIR:?set IKEMEN_DIR to the Ikemen-GO runtime directory}"
root="$(cd "$root" && pwd)"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
binary="${IKEMEN_BIN:-$root/ikemen-local}"
scenario="${IKEMEN_ORACLE_SCENARIO:-kfm_attack}"
p1_ai="${IKEMEN_ORACLE_P1_AI:-0}"
p2_ai="${IKEMEN_ORACLE_P2_AI:-0}"
seconds="${IKEMEN_ORACLE_SECONDS:-10}"
timeout_s="${IKEMEN_ORACLE_TIMEOUT:-14}"
main="$root/external/script/main.lua"
mod="$root/external/mods/zz-oracle.lua"
capture="$(mktemp -t manato-frames-XXXXXX.csv)"

cleanup() {
    if [[ -f "$main.bak" ]]; then
        mv "$main.bak" "$main"
    fi
    rm -f "$mod"
    if [[ "${IKEMEN_ORACLE_KEEP:-0}" != "1" ]]; then
        rm -f "$capture"
    fi
}
trap cleanup EXIT

cp "$here/capture.lua" "$mod"
cp "$here/oracle-stage.def" "$root/stages/oracle.def"
cp "$repo/content/stages/training/stage1.sff" "$root/stages/oracle.sff"

case "$scenario" in
    kaen_*|nival_*|ordan_*)
        char="${scenario%%_*}"
        rm -rf "$root/chars/$char"
        cp -r "$repo/content/chars/$char" "$root/chars/$char"
        export IKEMEN_ORACLE_P1="$char"
        ;;
esac

if [[ "$scenario" == "kfm_multihit" ]]; then
    rm -rf "$root/chars/oracle"
    cp -r "$root/chars/kfm" "$root/chars/oracle"
    mv "$root/chars/oracle/kfm.def" "$root/chars/oracle/oracle.def"
    python3 - "$root/chars/oracle/oracle.def" <<'PY'
import sys
from pathlib import Path
path = Path(sys.argv[1])
text = path.read_text()
if "oracle.cns" not in text:
    text = text.replace(
        "stcommon = common1.cns",
        "stcommon = common1.cns\nst2     = oracle.cns",
        1,
    )
    path.write_text(text)
PY
    cp "$here/oracle-char/oracle.cns" "$root/chars/oracle/oracle.cns"
    export IKEMEN_ORACLE_P1=oracle
    export IKEMEN_ORACLE_P2=oracle
fi

cp "$main" "$main.bak"
if ! grep -q "zz-oracle.lua" "$main"; then
    python3 - "$main" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()
needle = "--initiate quick match only if"
if needle not in text:
    raise SystemExit(f"{path} has no quick-match hook")
path.write_text(
    text.replace(
        needle,
        "pcall(function() dofile('external/mods/zz-oracle.lua') end)\n\n" + needle,
        1,
    )
)
PY
fi

set +e
(
    cd "$root"
    SDL_VIDEODRIVER="${SDL_VIDEODRIVER:-offscreen}" \
    LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}:$root/lib" \
    IKEMEN_ORACLE_OUT="$capture" \
    IKEMEN_ORACLE_SCENARIO="$scenario" \
    timeout "$timeout_s" "$binary" \
        -windowed -nosound -nomusic -nojoy -debug \
        -p1 "${IKEMEN_ORACLE_P1:-kfm}" -p2 "${IKEMEN_ORACLE_P2:-kfm}" \
        -p1.ai "$p1_ai" -p2.ai "$p2_ai" -s oracle -rounds 1 -time "$seconds" \
        >/dev/null 2>&1
)
status=$?
set -e

if [[ ! -s "$capture" ]]; then
    echo "oracle produced no per-frame capture (exit $status)" >&2
    exit 1
fi

python3 "$here/run_frame_capture.py" --from-csv "$capture" --json
if [[ "${IKEMEN_ORACLE_KEEP:-0}" == "1" ]]; then
    echo "raw capture: $capture" >&2
    trap - EXIT
    rm -f "$mod"
    [[ -f "$main.bak" ]] && mv "$main.bak" "$main"
fi
