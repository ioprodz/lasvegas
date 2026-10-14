// Listen controls: double clap toggles the lights (detected on the Pi's mic)
const listenClap = document.getElementById('listen-clap');
const listenSensitivity = document.getElementById('listen-sensitivity');
const listenSensitivityValue = document.getElementById('listen-sensitivity-value');
const listenMic = document.getElementById('listen-mic');

function sendListen() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send('listen:set:' + (listenClap.checked ? '1' : '0') + ',' + listenSensitivity.value);
}

// Populate the form from a server-sent config snapshot.
function applyListenFromServer(data) {
    if (typeof data.clap === 'boolean') listenClap.checked = data.clap;
    if (data.sensitivity != null) {
        listenSensitivity.value = data.sensitivity;
        listenSensitivityValue.textContent = data.sensitivity;
    }
    if (typeof data.mic === 'string') {
        listenMic.textContent = data.mic
            ? 'Listening on ' + data.mic + ' (reopened automatically at boot).'
            : 'No Pi mic selected: pick "HW" as the audio source first, clap detection uses it.';
    }
}

listenSensitivity.addEventListener('input', () => {
    listenSensitivityValue.textContent = listenSensitivity.value;
});
listenClap.addEventListener('change', sendListen);
listenSensitivity.addEventListener('change', sendListen);
