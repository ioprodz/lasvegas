#!/usr/bin/env bash
# Install or update the voice-command service on the Pi (uses your SSH key).
# Usage: ./ears/install.sh [host]   (default lasvegas.local)
set -euo pipefail

PI_HOST="${1:-${PI_HOST:-lasvegas.local}}"
PI_USER="freeu"
DIR="/home/${PI_USER}/ears"
MODEL="vosk-model-small-en-us-0.15"
SSH="ssh -o ConnectTimeout=5 ${PI_USER}@${PI_HOST}"
HERE="$(cd "$(dirname "$0")" && pwd)"

echo "==> Copying files to ${PI_HOST}:${DIR}"
${SSH} "mkdir -p ${DIR}"
scp -q "${HERE}/ears.py" "${HERE}/requirements.txt" "${HERE}/lasvegas-ears.service" "${PI_USER}@${PI_HOST}:${DIR}/"

echo "==> Python environment"
${SSH} "test -x ${DIR}/venv/bin/python || python3 -m venv ${DIR}/venv"
${SSH} "${DIR}/venv/bin/pip install -q -r ${DIR}/requirements.txt"

echo "==> Vosk model (${MODEL}, ~40 MB, from alphacephei.com)"
${SSH} "test -d ${DIR}/model || (cd ${DIR} && curl -fsSLO https://alphacephei.com/vosk/models/${MODEL}.zip \
    && python3 -m zipfile -e ${MODEL}.zip . && mv ${MODEL} model && rm ${MODEL}.zip)"

echo "==> systemd unit"
${SSH} "sudo cp ${DIR}/lasvegas-ears.service /etc/systemd/system/ && sudo systemctl daemon-reload \
    && sudo systemctl enable lasvegas-ears && sudo systemctl restart lasvegas-ears"
sleep 3
${SSH} "systemctl --no-pager status lasvegas-ears | head -5; journalctl -u lasvegas-ears -n 5 --no-pager -q"
