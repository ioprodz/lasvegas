// WebSocket connection management
const statusEl = document.getElementById('status');
let ws;

function connect() {
    const host = window.location.hostname;
    ws = new WebSocket(`ws://${host}:8080`);
    ws.binaryType = 'arraybuffer';
    ws.onopen = () => { statusEl.textContent = 'Online'; statusEl.className = 'connected'; };
    ws.onclose = () => {
        statusEl.textContent = 'Offline'; statusEl.className = 'disconnected';
        setActiveAnim(null);
        setTimeout(connect, 2000);
    };
    ws.onmessage = handleWsMessage;
}
