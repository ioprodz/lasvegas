// Network page UI — interfaces, AP, Wi-Fi client list, eth0, apply-and-revert.

let netState = null;       // last server snapshot
let netScanResults = [];   // last scan results
let netStageCountdownTimer = null;

function netRequestStatus() {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send('net:list');
}

function netWifiScan() {
    if (ws && ws.readyState === WebSocket.OPEN) {
        const btn = document.getElementById('net-scan-btn');
        if (btn) { btn.disabled = true; btn.textContent = 'Scanning...'; }
        ws.send('net:wifi:scan');
    }
}

function netWifiUpsert(w) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('net:wifi:upsert:' + JSON.stringify(w));
    }
}

function netWifiRemove(ssid) {
    if (!confirm('Remove saved Wi-Fi "' + ssid + '"?')) return;
    if (ws && ws.readyState === WebSocket.OPEN) ws.send('net:wifi:remove:' + ssid);
}

function netWifiConnect(ssid) {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send('net:wifi:connect:' + ssid);
}

function netApSave() {
    const ssid = document.getElementById('net-ap-ssid').value.trim();
    const password = document.getElementById('net-ap-password').value;
    const band = document.getElementById('net-ap-band').value;
    const channel = parseInt(document.getElementById('net-ap-channel').value, 10) || 0;
    const enabled = document.getElementById('net-ap-enabled').checked;
    if (!ssid) { netSetStatus('AP SSID cannot be empty'); return; }
    if (password.length > 0 && password.length < 8) { netSetStatus('AP password must be at least 8 characters'); return; }
    ws.send('net:ap:set:' + JSON.stringify({ ssid, password, band, channel, enabled }));
}

function netApToggle(enabled) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('net:ap:toggle:' + (enabled ? '1' : '0'));
    }
}

function netEthSave() {
    const mode = document.querySelector('input[name="net-eth-mode"]:checked').value;
    const enabled = document.getElementById('net-eth-enabled').checked;
    let payload;
    if (mode === 'static') {
        const ip = document.getElementById('net-eth-ip').value.trim();
        const prefix = parseInt(document.getElementById('net-eth-prefix').value, 10) || 24;
        const gateway = document.getElementById('net-eth-gateway').value.trim();
        const dns = document.getElementById('net-eth-dns').value.trim();
        if (!ip) { netSetStatus('Static IP cannot be empty'); return; }
        payload = { mode: 'static', ip, prefix, gateway, dns, enabled };
    } else {
        payload = { mode: 'dhcp', enabled };
    }
    if (!confirm('Apply Ethernet changes? You have 30 seconds to confirm or they will auto-revert.')) return;
    ws.send('net:eth:set:' + JSON.stringify(payload));
}

function netStageConfirm(token) {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send('net:stage:confirm:' + token);
}

function netStageRevert(token) {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send('net:stage:revert:' + token);
}

function netSetStatus(msg) {
    const el = document.getElementById('net-status-msg');
    if (!el) return;
    el.textContent = msg || '';
    el.style.display = msg ? 'block' : 'none';
}

// ── Rendering ─────────────────────────────────────────────────────────

function netEscape(s) {
    const div = document.createElement('div');
    div.textContent = s == null ? '' : String(s);
    return div.innerHTML;
}

function netRenderInterfaces(interfaces) {
    const list = document.getElementById('net-iface-list');
    if (!list) return;
    if (!interfaces || interfaces.length === 0) {
        list.innerHTML = '<div class="bt-empty">No interfaces.</div>';
        return;
    }
    list.innerHTML = interfaces.map(i => {
        const roleLabel = {
            'eth-console': 'Console (eth0)',
            'wifi-client': 'Wi-Fi client',
            'ap': 'Access Point',
        }[i.role] || i.kind;
        const stateBadge = i.state === 'connected'
            ? '<span class="bt-badge bt-badge-connected">' + netEscape(i.state) + '</span>'
            : '<span class="bt-badge bt-badge-new">' + netEscape(i.state) + '</span>';
        const extras = [];
        if (i.ssid) extras.push('SSID: ' + netEscape(i.ssid));
        if (i.ip4) extras.push('IP: ' + netEscape(i.ip4));
        if (i.kind === 'wifi' && i.signal) extras.push('Signal: ' + i.signal + '%');
        return `<div class="bt-device">
            <div class="bt-device-info">
                <div class="bt-device-name">${netEscape(i.name)} <span style="color:#777;font-weight:400;font-size:0.8em;">— ${netEscape(roleLabel)}</span></div>
                <div class="bt-device-mac">${extras.join(' · ') || '—'}</div>
            </div>
            <div class="bt-device-status">${stateBadge}</div>
        </div>`;
    }).join('');
}

function netRenderAp(ap) {
    if (!ap) return;
    const ssid = document.getElementById('net-ap-ssid');
    const password = document.getElementById('net-ap-password');
    const band = document.getElementById('net-ap-band');
    const channel = document.getElementById('net-ap-channel');
    const enabled = document.getElementById('net-ap-enabled');
    // Only update fields the user isn't editing (avoid clobbering typing).
    if (document.activeElement !== ssid) ssid.value = ap.ssid || '';
    if (document.activeElement !== password) password.value = ap.password || '';
    if (document.activeElement !== band) band.value = ap.band || 'bg';
    if (document.activeElement !== channel) channel.value = ap.channel || 0;
    enabled.checked = !!ap.enabled;

    const status = document.getElementById('net-ap-iface-status');
    if (status) {
        if (ap.iface_present) {
            status.textContent = 'TP-Link adapter detected (wlan_ap)';
            status.style.color = '#4ecdc4';
        } else {
            status.textContent = 'TP-Link adapter not detected — plug in to activate AP';
            status.style.color = '#cfcf6f';
        }
    }
}

function netRenderApClients(clients) {
    const list = document.getElementById('net-ap-clients');
    if (!list) return;
    if (!clients || clients.length === 0) {
        list.innerHTML = '<div class="bt-empty">No clients connected to AP.</div>';
        return;
    }
    list.innerHTML = clients.map(c => `<div class="bt-device">
        <div class="bt-device-info">
            <div class="bt-device-name">${netEscape(c.hostname || '(unknown)')}</div>
            <div class="bt-device-mac">${netEscape(c.mac)}${c.ip ? ' · ' + netEscape(c.ip) : ''}</div>
        </div>
    </div>`).join('');
}

function netRenderKnownWifis(wifis) {
    const list = document.getElementById('net-known-wifi-list');
    if (!list) return;
    if (!wifis || wifis.length === 0) {
        list.innerHTML = '<div class="bt-empty">No saved networks. Scan or add one below.</div>';
        return;
    }
    // Sort by priority desc, then ssid
    const sorted = [...wifis].sort((a, b) => (b.priority - a.priority) || a.ssid.localeCompare(b.ssid));
    list.innerHTML = sorted.map(w => `<div class="bt-device">
        <div class="bt-device-info">
            <div class="bt-device-name">${netEscape(w.ssid)}${w.hidden ? ' <span style="color:#777;font-size:0.8em;">(hidden)</span>' : ''}</div>
            <div class="bt-device-mac">priority ${w.priority} · ${w.has_password ? 'WPA-PSK' : 'open'}</div>
        </div>
        <div class="bt-device-actions">
            <button class="bt-action-btn bt-action-connect" onclick="netWifiConnect('${netEscape(w.ssid).replace(/'/g, '&#39;')}')">Connect</button>
            <button class="bt-action-btn bt-action-pair" onclick="netEditKnown('${netEscape(w.ssid).replace(/'/g, '&#39;')}')">Edit</button>
            <button class="bt-action-btn bt-action-remove" onclick="netWifiRemove('${netEscape(w.ssid).replace(/'/g, '&#39;')}')">Remove</button>
        </div>
    </div>`).join('');
}

function netRenderScan(aps) {
    const list = document.getElementById('net-scan-list');
    if (!list) return;
    if (!aps || aps.length === 0) {
        list.innerHTML = '<div class="bt-empty">No scan results yet. Click "Scan".</div>';
        return;
    }
    list.innerHTML = aps.map(a => `<div class="bt-device">
        <div class="bt-device-info">
            <div class="bt-device-name">${netEscape(a.ssid)}${a.in_use ? ' <span class="bt-badge bt-badge-connected" style="margin-left:0.4rem;">in use</span>' : ''}</div>
            <div class="bt-device-mac">signal ${a.signal}% · ${netEscape(a.security || 'open')}</div>
        </div>
        <div class="bt-device-actions">
            <button class="bt-action-btn bt-action-pair" onclick="netAddFromScan('${netEscape(a.ssid).replace(/'/g, '&#39;')}', '${netEscape(a.security).replace(/'/g, '&#39;')}')">Add</button>
        </div>
    </div>`).join('');
}

function netRenderEth(eth) {
    if (!eth) return;
    const enabled = document.getElementById('net-eth-enabled');
    enabled.checked = !!eth.enabled;
    if (eth.mode === 'static') {
        document.getElementById('net-eth-mode-static').checked = true;
        document.getElementById('net-eth-ip').value = eth.ip || '';
        document.getElementById('net-eth-prefix').value = eth.prefix || 24;
        document.getElementById('net-eth-gateway').value = eth.gateway || '';
        document.getElementById('net-eth-dns').value = eth.dns || '';
    } else {
        document.getElementById('net-eth-mode-dhcp').checked = true;
    }
    netUpdateEthStaticVisibility();
}

function netUpdateEthStaticVisibility() {
    const isStatic = document.getElementById('net-eth-mode-static').checked;
    document.getElementById('net-eth-static-fields').style.display = isStatic ? 'block' : 'none';
}

function netRenderPending(pending) {
    const banner = document.getElementById('net-stage-banner');
    if (!banner) return;
    if (netStageCountdownTimer) {
        clearInterval(netStageCountdownTimer);
        netStageCountdownTimer = null;
    }
    if (!pending) {
        banner.style.display = 'none';
        return;
    }
    banner.style.display = 'flex';
    let remaining = pending.timeout_secs || 30;
    const update = () => {
        banner.innerHTML = `<span>⚠ Network change pending on <b>${netEscape(pending.profile)}</b> — auto-revert in <span id="net-stage-secs">${remaining}</span>s.</span>
            <span class="bt-device-actions">
                <button class="bt-action-btn bt-action-connect" onclick="netStageConfirm(${pending.token})">Keep changes</button>
                <button class="bt-action-btn bt-action-remove" onclick="netStageRevert(${pending.token})">Revert now</button>
            </span>`;
    };
    update();
    netStageCountdownTimer = setInterval(() => {
        remaining -= 1;
        const el = document.getElementById('net-stage-secs');
        if (el) el.textContent = remaining;
        if (remaining <= 0) {
            clearInterval(netStageCountdownTimer);
            netStageCountdownTimer = null;
        }
    }, 1000);
}

// ── Form helpers ──────────────────────────────────────────────────────

function netAddFromScan(ssid, security) {
    document.getElementById('net-new-wifi-ssid').value = ssid;
    document.getElementById('net-new-wifi-password').value = '';
    document.getElementById('net-new-wifi-hidden').checked = false;
    document.getElementById('net-new-wifi-priority').value = '0';
    document.getElementById('net-new-wifi-password').focus();
}

function netEditKnown(ssid) {
    document.getElementById('net-new-wifi-ssid').value = ssid;
    document.getElementById('net-new-wifi-password').value = '';
    const w = (netState && netState.known_wifis || []).find(x => x.ssid === ssid);
    if (w) {
        document.getElementById('net-new-wifi-priority').value = w.priority || 0;
        document.getElementById('net-new-wifi-hidden').checked = !!w.hidden;
    }
    document.getElementById('net-new-wifi-password').focus();
}

function netSubmitNewWifi() {
    const ssid = document.getElementById('net-new-wifi-ssid').value.trim();
    const password = document.getElementById('net-new-wifi-password').value;
    const priority = parseInt(document.getElementById('net-new-wifi-priority').value, 10) || 0;
    const hidden = document.getElementById('net-new-wifi-hidden').checked;
    if (!ssid) { netSetStatus('SSID required'); return; }
    netWifiUpsert({ ssid, password, priority, hidden });
    document.getElementById('net-new-wifi-password').value = '';
}

// ── Message handlers (called from app.js dispatcher) ───────────────────

function handleNetStatus(json) {
    try {
        const s = JSON.parse(json);
        netState = s;
        netRenderInterfaces(s.interfaces);
        netRenderAp(s.ap);
        netRenderApClients(s.ap_clients);
        netRenderKnownWifis(s.known_wifis);
        netRenderEth(s.eth);
        netRenderPending(s.pending);
    } catch (e) {
        console.error('Failed to parse net status', e);
    }
}

function handleNetResult(result) {
    // Examples: ap:set:ok, ap:set:error:msg, wifi:scan:[json], stage:pending:<token>:<secs>
    if (result.startsWith('wifi:scan:')) {
        const json = result.slice('wifi:scan:'.length);
        try {
            netScanResults = JSON.parse(json);
            netRenderScan(netScanResults);
        } catch (e) {
            console.error('Failed to parse wifi scan', e);
        }
        const btn = document.getElementById('net-scan-btn');
        if (btn) { btn.disabled = false; btn.textContent = 'Scan'; }
        return;
    }
    if (result.startsWith('stage:pending:')) {
        // Optimistic: server will also push NetStatus with pending; just flash a status line.
        const parts = result.split(':');
        netSetStatus('Change applied — confirm within ' + parts[3] + 's or auto-revert.');
        setTimeout(() => netSetStatus(''), 5000);
        return;
    }
    if (result.startsWith('stage:confirm:')) {
        netSetStatus('Change kept.');
        setTimeout(() => netSetStatus(''), 3000);
        return;
    }
    if (result.startsWith('stage:revert:')) {
        netSetStatus('Change reverted.');
        setTimeout(() => netSetStatus(''), 3000);
        return;
    }
    if (result.endsWith(':ok') || result.includes(':ok:')) {
        netSetStatus('OK');
        setTimeout(() => netSetStatus(''), 2500);
        return;
    }
    if (result.includes(':error')) {
        const idx = result.indexOf(':error');
        netSetStatus('Error: ' + result.slice(idx + ':error'.length).replace(/^:/, ''));
        return;
    }
}

// ── Event wiring (runs at script load; DOM ready by then since script is in <body>) ──

(function netWireUp() {
    const scanBtn = document.getElementById('net-scan-btn');
    if (scanBtn) scanBtn.addEventListener('click', netWifiScan);
    const refreshBtn = document.getElementById('net-refresh-btn');
    if (refreshBtn) refreshBtn.addEventListener('click', netRequestStatus);
    const apSaveBtn = document.getElementById('net-ap-save-btn');
    if (apSaveBtn) apSaveBtn.addEventListener('click', netApSave);
    const apToggle = document.getElementById('net-ap-enabled');
    if (apToggle) apToggle.addEventListener('change', e => netApToggle(e.target.checked));
    const ethSaveBtn = document.getElementById('net-eth-save-btn');
    if (ethSaveBtn) ethSaveBtn.addEventListener('click', netEthSave);
    const addWifiBtn = document.getElementById('net-add-wifi-btn');
    if (addWifiBtn) addWifiBtn.addEventListener('click', netSubmitNewWifi);
    document.querySelectorAll('input[name="net-eth-mode"]').forEach(r => {
        r.addEventListener('change', netUpdateEthStaticVisibility);
    });
})();
