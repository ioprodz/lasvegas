#!/usr/bin/env bash
set -e

PI_HOST="192.168.1.97"
PI_USER="freeu"
PI_PASS="freepass"
TARGET="aarch64-unknown-linux-musl"
BINARY="lasvegas"
REMOTE_DIR="/home/${PI_USER}"

SSH="sshpass -p ${PI_PASS} ssh -o StrictHostKeyChecking=no ${PI_USER}@${PI_HOST}"
SCP="sshpass -p ${PI_PASS} scp -o StrictHostKeyChecking=no"

echo "==> Building for ${TARGET}..."
cross build --target ${TARGET} --release

echo "==> Stopping any running instance on Pi..."
${SSH} "sudo killall ${BINARY} 2>/dev/null || true"

echo "==> Deploying binary to Pi..."
${SCP} target/${TARGET}/release/${BINARY} ${PI_USER}@${PI_HOST}:${REMOTE_DIR}/${BINARY}

echo "==> Running on Pi..."
${SSH} "sudo ${REMOTE_DIR}/${BINARY}"
