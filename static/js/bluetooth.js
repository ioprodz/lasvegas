// Bluetooth device management UI

function btScan() {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('bt:scan');
    }
}

function btRefresh() {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('bt:list');
    }
}

function btPair(mac) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('bt:pair:' + mac);
        btSetStatus('Pairing with ' + mac + '...');
    }
}

function btConnect(mac) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('bt:connect:' + mac);
        btSetStatus('Connecting to ' + mac + '...');
    }
}

function btDisconnect(mac) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('bt:disconnect:' + mac);
        btSetStatus('Disconnecting ' + mac + '...');
    }
}

function btRemove(mac) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('bt:remove:' + mac);
        btSetStatus('Removing ' + mac + '...');
    }
}

function btSetStatus(msg) {
    const el = document.getElementById('bt-status');
    el.textContent = msg;
    el.style.display = msg ? 'block' : 'none';
}

function btRenderDevices(devices) {
    const list = document.getElementById('bt-device-list');

    if (!devices || devices.length === 0) {
        list.innerHTML = '<div class="bt-empty">No devices found. Click "Scan for Devices" to discover nearby Bluetooth devices.</div>';
        return;
    }

    list.innerHTML = devices.map(d => {
        const statusBadge = d.connected
            ? '<span class="bt-badge bt-badge-connected">Connected</span>'
            : d.paired
                ? '<span class="bt-badge bt-badge-paired">Paired</span>'
                : '<span class="bt-badge bt-badge-new">New</span>';

        let actions = '';
        if (d.connected) {
            actions = `<button class="bt-action-btn bt-action-disconnect" onclick="btDisconnect('${d.mac}')">Disconnect</button>`;
        } else if (d.paired) {
            actions = `<button class="bt-action-btn bt-action-connect" onclick="btConnect('${d.mac}')">Connect</button>`
                + `<button class="bt-action-btn bt-action-remove" onclick="btRemove('${d.mac}')">Remove</button>`;
        } else {
            actions = `<button class="bt-action-btn bt-action-pair" onclick="btPair('${d.mac}')">Pair</button>`;
        }

        return `<div class="bt-device">
            <div class="bt-device-info">
                <div class="bt-device-name">${escapeHtml(d.name)}</div>
                <div class="bt-device-mac">${escapeHtml(d.mac)}</div>
            </div>
            <div class="bt-device-status">${statusBadge}</div>
            <div class="bt-device-actions">${actions}</div>
        </div>`;
    }).join('');
}

function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}

function handleBtDeviceList(json) {
    try {
        const devices = JSON.parse(json);
        btRenderDevices(devices);
    } catch (e) {
        console.error('Failed to parse BT device list:', e);
    }
}

function handleBtResult(result) {
    // "scan:scanning", "scan:ok", "pair:ok:MAC", "pair:error:MAC:msg", etc.
    const parts = result.split(':');
    const action = parts[0];
    const status = parts[1];

    if (action === 'scan' && status === 'scanning') {
        btSetStatus('Scanning for Bluetooth devices...');
        document.getElementById('bt-scan-btn').disabled = true;
        document.getElementById('bt-scan-btn').textContent = 'Scanning...';
    } else if (action === 'scan' && status === 'ok') {
        btSetStatus('Scan complete.');
        document.getElementById('bt-scan-btn').disabled = false;
        document.getElementById('bt-scan-btn').textContent = 'Scan for Devices';
        setTimeout(() => btSetStatus(''), 3000);
    } else if (status === 'ok') {
        btSetStatus(action.charAt(0).toUpperCase() + action.slice(1) + ' successful.');
        setTimeout(() => btSetStatus(''), 3000);
    } else if (status === 'error') {
        const msg = parts.slice(3).join(':');
        btSetStatus('Error: ' + msg);
    }
}
