// App entry point -- wires everything together

// Hardware audio state
let hwAudioActive = false;

// WebSocket message handler (defined here because it depends on globals from multiple modules)
function handleWsMessage(event) {
    if (event.data instanceof ArrayBuffer) {
        const data = new Uint8Array(event.data);
        // 0x04 = hardware audio analysis streamed from Pi
        if (data.length === 20 && data[0] === 0x04) {
            handleHwAudioData(data);
        } else {
            // LED state (raw RGB, no prefix byte)
            updateGrid(data);
        }
    } else if (typeof event.data === 'string') {
        if (event.data.startsWith('calibration:')) {
            try {
                applyCalibrationFromServer(JSON.parse(event.data.slice('calibration:'.length)));
            } catch(e) { console.error('Failed to parse calibration:', e); }
        } else if (event.data.startsWith('hw_audio:devices:')) {
            handleHwDeviceList(event.data.slice('hw_audio:devices:'.length));
        } else if (event.data.startsWith('hw_audio:status:')) {
            handleHwStatus(event.data.slice('hw_audio:status:'.length));
        } else if (event.data.startsWith('bt:devices:')) {
            handleBtDeviceList(event.data.slice('bt:devices:'.length));
        } else if (event.data.startsWith('bt:result:')) {
            handleBtResult(event.data.slice('bt:result:'.length));
        } else if (event.data.startsWith('net:status:')) {
            handleNetStatus(event.data.slice('net:status:'.length));
        } else if (event.data.startsWith('net:result:')) {
            handleNetResult(event.data.slice('net:result:'.length));
        }
    }
}

// Handle hardware audio analysis data (0x04 binary, same layout as 0x03)
function handleHwAudioData(data) {
    const bands = data.slice(1, 9);
    // Update instrument indicators (same globals as browser audio)
    indKick = data[9] / 255;
    indSnare = data[10] / 255;
    indHihat = data[11] / 255;
    indVocals = data[12] / 255;
    indBassLine = data[13] / 255;
    const bpm = (data[14] << 8) | data[15];
    estimatedBPM = bpm;
    beatPhase = data[16] / 255;
    const midiNote = data[17];
    const chordRoot = data[18];
    const chordQuality = data[19];

    // Update pitch detection globals
    if (midiNote > 0) {
        detectedFreq = 440 * Math.pow(2, (midiNote - 69) / 12);
        const noteIdx = ((midiNote % 12) + 12) % 12;
        const octave = Math.floor(midiNote / 12) - 1;
        detectedNote = NOTE_NAMES[noteIdx] + octave;
    } else {
        detectedFreq = 0;
        detectedNote = '\u2014';
    }

    // Reconstruct chord name from root + quality
    if (chordRoot < 12 && chordQuality !== 255) {
        const rootName = NOTE_NAMES[chordRoot];
        const qualNames = ['maj', 'm', 'dim', 'aug', '7', 'maj7', 'm7', 'sus2', 'sus4', '5'];
        detectedChord = rootName + (chordQuality < qualNames.length ? ' ' + qualNames[chordQuality] : '?');
    } else {
        detectedChord = '\u2014';
    }

    // Drive the visualizer
    if (document.visibilityState === 'visible') {
        drawVisualizer(bands);
        updateChromaticCircle();
    }
}

// Handle hardware device list from server
function handleHwDeviceList(json) {
    try {
        const devices = JSON.parse(json);
        console.log('handleHwDeviceList: received', devices.length, 'devices', json.substring(0, 200));
        const select = document.getElementById('hw-audio-device');
        select.innerHTML = '';
        if (devices.length === 0) {
            select.innerHTML = '<option value="">No devices found</option>';
            select.disabled = true;
            document.getElementById('btn-hw-start').disabled = true;
        } else {
            devices.forEach((d, i) => {
                const opt = document.createElement('option');
                opt.value = d.id;
                opt.textContent = d.name || d.id;
                if (i === 0) opt.selected = true;
                select.appendChild(opt);
            });
            select.disabled = false;
            document.getElementById('btn-hw-start').disabled = false;
            console.log('handleHwDeviceList: first option value=' + JSON.stringify(select.value));
        }
    } catch (e) {
        console.error('Failed to parse device list:', e);
    }
}

// Handle hardware audio status messages
function handleHwStatus(status) {
    const startBtn = document.getElementById('btn-hw-start');
    const stopBtn = document.getElementById('btn-hw-stop');
    const select = document.getElementById('hw-audio-device');

    if (status === 'started') {
        hwAudioActive = true;
        startBtn.disabled = true;
        stopBtn.disabled = false;
        select.disabled = true;
        startBtn.classList.add('active');
        // Disable browser audio buttons
        document.getElementById('btn-mic').disabled = true;
        document.getElementById('btn-system').disabled = true;
    } else if (status === 'stopped') {
        hwAudioActive = false;
        startBtn.disabled = false;
        stopBtn.disabled = true;
        select.disabled = false;
        startBtn.classList.remove('active');
        // Re-enable browser audio buttons
        document.getElementById('btn-mic').disabled = false;
        document.getElementById('btn-system').disabled = false;
    } else if (status.startsWith('error:')) {
        hwAudioActive = false;
        startBtn.disabled = false;
        stopBtn.disabled = true;
        select.disabled = false;
        startBtn.classList.remove('active');
        document.getElementById('btn-mic').disabled = false;
        document.getElementById('btn-system').disabled = false;
        console.error('Hardware audio error:', status.slice(6));
    }
}

// Hardware audio controls
function startHwAudio() {
    const select = document.getElementById('hw-audio-device');
    const deviceId = select.value;
    console.log('startHwAudio: deviceId=' + JSON.stringify(deviceId) + ' options=' + select.options.length + ' disabled=' + select.disabled);
    if (!deviceId) {
        console.warn('startHwAudio: no device selected');
        return;
    }
    // Stop browser audio first (mutual exclusion)
    stopAudio();
    if (ws && ws.readyState === WebSocket.OPEN) {
        console.log('startHwAudio: sending hw_audio:start:' + deviceId);
        ws.send('hw_audio:start:' + deviceId);
    }
}

function stopHwAudio() {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('hw_audio:stop');
    }
}

function requestHwDevices() {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('hw_audio:list');
    }
}

// Audio button listeners
document.getElementById('btn-mic').addEventListener('click', () => {
    // Stop hardware audio if active (mutual exclusion)
    if (hwAudioActive) stopHwAudio();
    startAudio('mic');
});
document.getElementById('btn-system').addEventListener('click', () => {
    if (hwAudioActive) stopHwAudio();
    startAudio('system');
});
document.getElementById('btn-stop-audio').addEventListener('click', stopAudio);

// Hardware audio button listeners
document.getElementById('btn-hw-start').addEventListener('click', startHwAudio);
document.getElementById('btn-hw-stop').addEventListener('click', stopHwAudio);

// Bluetooth button listeners
document.getElementById('bt-scan-btn').addEventListener('click', btScan);
document.getElementById('bt-refresh-btn').addEventListener('click', btRefresh);

// Route from current URL path, then connect WebSocket
routeFromPath();
connect();
