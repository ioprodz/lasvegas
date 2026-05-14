#!/usr/bin/env bash
set -e

# Override the target with: PI_HOST=<ip-or-host> ./deploy.sh   or   ./deploy.sh <ip-or-host>
# Default uses mDNS so the script works regardless of which interface (eth/wifi)
# the Pi is reachable on, and regardless of the DHCP-assigned IP.
PI_HOST="${PI_HOST:-${1:-lasvegas.local}}"
PI_USER="freeu"
PI_PASS="freepass"
TARGET="aarch64-unknown-linux-musl"
BINARY="lasvegas"
REMOTE_DIR="/home/${PI_USER}"
SERVICE_NAME="lasvegas"
HOSTNAME_NEW="lasvegas"
PROVISION_MARKER="/etc/lasvegas.provisioned"

SSH="sshpass -p ${PI_PASS} ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 ${PI_USER}@${PI_HOST}"
SCP="sshpass -p ${PI_PASS} scp -o StrictHostKeyChecking=no -o ConnectTimeout=5"

echo "==> Target: ${PI_HOST}"
if ! ${SSH} 'true' 2>/dev/null; then
    echo "ERROR: cannot reach ${PI_HOST} via SSH."
    if [ "${PI_HOST}" = "lasvegas.local" ]; then
        echo "  mDNS resolution failed. Try the LAN IP instead, e.g.:"
        echo "    ./deploy.sh 192.168.1.97"
        echo "  Once you've deployed once via IP, mDNS should work for subsequent runs."
    fi
    exit 1
fi

echo "==> Building for ${TARGET}..."
cross build --target ${TARGET} --release

# ── One-time provisioning (NetworkManager, avahi, hostname, TP-Link udev) ─────
if ! ${SSH} "test -f ${PROVISION_MARKER}"; then
    echo "==> First-run provisioning: installing NetworkManager + avahi, setting hostname, detecting AP adapter..."

    # 1. NetworkManager (idempotent install + activate; disable dhcpcd if it's the active stack).
    ${SSH} "sudo apt-get update && sudo apt-get install -y network-manager avahi-daemon"
    ${SSH} "sudo systemctl disable --now dhcpcd 2>/dev/null || true"
    ${SSH} "sudo systemctl enable --now NetworkManager"
    ${SSH} "sudo systemctl enable --now avahi-daemon"

    # 2. Hostname + mDNS.
    ${SSH} "sudo hostnamectl set-hostname ${HOSTNAME_NEW}"
    # Update /etc/hosts so sudo doesn't warn about an unresolvable hostname.
    ${SSH} "sudo sed -i 's/^127\\.0\\.1\\.1.*/127.0.1.1\\t${HOSTNAME_NEW}/' /etc/hosts || echo '127.0.1.1\\t${HOSTNAME_NEW}' | sudo tee -a /etc/hosts > /dev/null"

    # Lock the hostname against the three things on Raspberry Pi OS Bookworm that try to rewrite it:
    #   - cloud-init's update_hostname module (runs on every boot)
    #   - NetworkManager's hostname-mode=dhcp (picks up DHCP option 12 from the router; Livebox sends its own name)
    #   - DHCP send-hostname round-tripping
    ${SSH} "sudo tee /etc/cloud/cloud.cfg.d/99-lasvegas-hostname.cfg > /dev/null" <<'EOF'
preserve_hostname: true
EOF
    ${SSH} "sudo tee /etc/NetworkManager/conf.d/10-hostname.conf > /dev/null" <<'EOF'
[main]
hostname-mode=none
EOF
    ${SSH} "sudo systemctl reload NetworkManager 2>/dev/null || sudo systemctl restart NetworkManager"

    # Restart avahi so it picks up the new hostname (it caches the kernel hostname at startup).
    ${SSH} "sudo systemctl restart avahi-daemon"

    # 3. Auto-detect TP-Link USB Wi-Fi adapter (any USB-bound wireless that isn't onboard brcmfmac).
    #    Find its MAC, write a systemd .link file to persistently rename it to wlan_ap,
    #    and if the adapter is already plugged in, rename it live right now too.
    echo "==> Detecting USB Wi-Fi adapter..."
    ADAPTER_INFO=$(${SSH} '
        for path in /sys/class/net/wlan* /sys/class/net/wlan_ap; do
            [ -e "$path" ] || continue
            ifname=$(basename "$path")
            driver=$(readlink -f "$path/device/driver" 2>/dev/null | xargs -I{} basename {} 2>/dev/null)
            # Skip onboard Pi Wi-Fi (brcmfmac)
            [ "$driver" = "brcmfmac" ] && continue
            mac=$(cat "$path/address" 2>/dev/null)
            echo "$ifname $mac"
            exit 0
        done
    ' || true)

    if [ -n "${ADAPTER_INFO}" ]; then
        CURRENT_NAME=$(echo "${ADAPTER_INFO}" | awk "{print \$1}")
        ADAPTER_MAC=$(echo "${ADAPTER_INFO}" | awk "{print \$2}")
        echo "==> Found USB Wi-Fi: ${CURRENT_NAME} (${ADAPTER_MAC})"

        echo "==> Installing systemd .link file..."
        ${SSH} "sudo tee /etc/systemd/network/10-lasvegas-ap.link > /dev/null" <<EOF
[Match]
MACAddress=${ADAPTER_MAC}

[Link]
Name=wlan_ap
EOF

        # Clean up any older udev rule from previous deploys.
        ${SSH} "sudo rm -f /etc/udev/rules.d/80-lasvegas-ap.rules"
        ${SSH} "sudo udevadm control --reload"

        # If the interface isn't already wlan_ap, rename it live so we don't need a reboot.
        if [ "${CURRENT_NAME}" != "wlan_ap" ]; then
            echo "==> Renaming ${CURRENT_NAME} -> wlan_ap..."
            ${SSH} "sudo nmcli dev set ${CURRENT_NAME} managed no || true"
            ${SSH} "sudo ip link set ${CURRENT_NAME} down"
            ${SSH} "sudo ip link set ${CURRENT_NAME} name wlan_ap"
            ${SSH} "sudo ip link set wlan_ap up"
            ${SSH} "sudo nmcli dev set wlan_ap managed yes || true"
        fi

        # 4. Pre-seed AP profile so it auto-activates the moment wlan_ap appears.
        #    Generate a random 12-char password using only chars the Rust code accepts.
        AP_PASS=$(${SSH} "head /dev/urandom | tr -dc 'a-km-zA-HJ-NP-Z2-9' | head -c 12")
        echo "==> Seeding AP profile lv-ap (password: ${AP_PASS})"
        ${SSH} "sudo nmcli con delete lv-ap 2>/dev/null || true"
        ${SSH} "sudo nmcli con add type wifi ifname wlan_ap con-name lv-ap autoconnect yes ssid lasvegas -- \
            802-11-wireless.mode ap 802-11-wireless.band bg \
            ipv4.method shared ipv6.method ignore \
            wifi-sec.key-mgmt wpa-psk wifi-sec.psk '${AP_PASS}'"
        # Persist into the app's network.json so the UI shows the password on first open.
        ${SSH} "cat > ${REMOTE_DIR}/network.json" <<EOF
{"ap":{"ssid":"lasvegas","password":"${AP_PASS}","band":"bg","channel":0,"enabled":true},"known_wifis":[]}
EOF
        ${SSH} "sudo chown ${PI_USER}:${PI_USER} ${REMOTE_DIR}/network.json"
    else
        echo "==> No external USB Wi-Fi adapter detected at provisioning time."
        echo "    Plug in the TP-Link and re-run deploy.sh, or use the Network page to detect later."
    fi

    # 5. Mark provisioned.
    ${SSH} "sudo touch ${PROVISION_MARKER}"
    echo "==> Provisioning complete."
else
    echo "==> Provisioning already done (${PROVISION_MARKER} exists). Skipping."
fi

echo "==> Stopping service on Pi (if running)..."
${SSH} "sudo systemctl stop ${SERVICE_NAME} 2>/dev/null || true; sudo killall ${BINARY} 2>/dev/null || true"

echo "==> Deploying binary to Pi..."
${SCP} target/${TARGET}/release/${BINARY} ${PI_USER}@${PI_HOST}:${REMOTE_DIR}/${BINARY}
${SSH} "chmod +x ${REMOTE_DIR}/${BINARY}"

echo "==> Installing systemd unit..."
UNIT_CONTENT="[Unit]
Description=Las Vegas LED Server
After=network-online.target bluetooth.target NetworkManager.service
Wants=network-online.target NetworkManager.service

[Service]
Type=simple
ExecStart=${REMOTE_DIR}/${BINARY}
WorkingDirectory=${REMOTE_DIR}
Restart=always
RestartSec=2
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
"
${SSH} "echo '${UNIT_CONTENT}' | sudo tee /etc/systemd/system/${SERVICE_NAME}.service > /dev/null"

echo "==> Enabling and starting service..."
${SSH} "sudo systemctl daemon-reload && sudo systemctl enable ${SERVICE_NAME} && sudo systemctl restart ${SERVICE_NAME}"

echo "==> Service status:"
${SSH} "sudo systemctl --no-pager status ${SERVICE_NAME} || true"

echo ""
echo "==> UI: http://${PI_HOST}"
echo ""
echo "==> Tailing logs (Ctrl+C to detach; service keeps running)..."
${SSH} "sudo journalctl -u ${SERVICE_NAME} -f -n 20"
