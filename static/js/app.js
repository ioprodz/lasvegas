// App entry point -- wires everything together

// WebSocket message handler (defined here because it depends on globals from multiple modules)
function handleWsMessage(event) {
    if (event.data instanceof ArrayBuffer) {
        updateGrid(new Uint8Array(event.data));
    } else if (typeof event.data === 'string' && event.data.startsWith('calibration:')) {
        try {
            applyCalibrationFromServer(JSON.parse(event.data.slice('calibration:'.length)));
        } catch(e) { console.error('Failed to parse calibration:', e); }
    }
}

// Audio button listeners
document.getElementById('btn-mic').addEventListener('click', () => startAudio('mic'));
document.getElementById('btn-system').addEventListener('click', () => startAudio('system'));
document.getElementById('btn-stop-audio').addEventListener('click', stopAudio);

// Route from current URL path, then connect WebSocket
routeFromPath();
connect();
