// Art-Net output controls
const artnetEnabled = document.getElementById('artnet-enabled');
const artnetDestIp = document.getElementById('artnet-dest-ip');
const artnetPort = document.getElementById('artnet-port');
const artnetUniverse = document.getElementById('artnet-universe');
const artnetDemo = document.getElementById('artnet-demo');
const artnetRig = document.getElementById('artnet-rig');

function sendArtNet() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const params = [
        artnetEnabled.checked ? '1' : '0',
        (artnetDestIp.value || '192.168.1.49').trim(),
        parseInt(artnetPort.value) || 6454,
        parseInt(artnetUniverse.value) || 0,
        artnetDemo.checked ? '1' : '0',
        artnetRig.checked ? '1' : '0',
    ].join(',');
    ws.send('artnet:set:' + params);
}

// Demo and Rig are mutually exclusive output modes.
artnetDemo.addEventListener('change', () => { if (artnetDemo.checked) artnetRig.checked = false; });
artnetRig.addEventListener('change', () => { if (artnetRig.checked) artnetDemo.checked = false; });

// Populate the form from a server-sent config snapshot.
function applyArtNetFromServer(data) {
    if (typeof data.enabled === 'boolean') artnetEnabled.checked = data.enabled;
    if (typeof data.dest_ip === 'string') artnetDestIp.value = data.dest_ip;
    if (data.port != null) artnetPort.value = data.port;
    if (data.universe != null) artnetUniverse.value = data.universe;
    if (typeof data.demo === 'boolean') artnetDemo.checked = data.demo;
    if (typeof data.rig === 'boolean') artnetRig.checked = data.rig;
}

document.getElementById('artnet-save-btn').addEventListener('click', sendArtNet);
