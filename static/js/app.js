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
        } else if (event.data.startsWith('shortcuts:')) {
            if (typeof applyShortcuts === 'function') {
                applyShortcuts(event.data.slice('shortcuts:'.length));
            }
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
        } else if (event.data.startsWith('artnet:')) {
            try {
                applyArtNetFromServer(JSON.parse(event.data.slice('artnet:'.length)));
            } catch(e) { console.error('Failed to parse artnet:', e); }
        } else if (event.data.startsWith('listen:')) {
            try {
                applyListenFromServer(JSON.parse(event.data.slice('listen:'.length)));
            } catch(e) { console.error('Failed to parse listen:', e); }
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

// Hardware audio device list — JS-managed (replaces the old hidden <select>).
let hwDevices = [];                       // [{id, name}, ...]
let selectedHwDeviceId = null;            // currently-chosen device id
try {
    const saved = localStorage.getItem('lv.hwDevice');
    if (saved) selectedHwDeviceId = saved;
} catch (_) {}

function setHwButtonLabel(name) {
    const label = document.getElementById('audio-mode-hw-label');
    if (label) label.textContent = name || 'HW Input';
}

function renderHwDeviceMenu() {
    const menu = document.getElementById('hw-device-menu');
    if (!menu) return;
    menu.innerHTML = '';
    if (hwDevices.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'hw-device-empty';
        empty.textContent = 'No devices found';
        menu.appendChild(empty);
        return;
    }
    hwDevices.forEach(d => {
        const item = document.createElement('button');
        item.type = 'button';
        item.className = 'hw-device-item';
        item.dataset.id = d.id;
        item.textContent = d.name || d.id;
        if (d.id === selectedHwDeviceId) item.classList.add('selected');
        item.addEventListener('click', () => pickHwDevice(d.id));
        menu.appendChild(item);
    });
}

function pickHwDevice(id) {
    selectedHwDeviceId = id;
    try { localStorage.setItem('lv.hwDevice', id); } catch (_) {}
    const dev = hwDevices.find(d => d.id === id);
    setHwButtonLabel(dev ? dev.name : 'HW Input');
    closeHwMenu();
    switchAudioSource('hw');
}

function openHwMenu() {
    const menu = document.getElementById('hw-device-menu');
    const btn = document.getElementById('audio-mode-hw');
    if (!menu) return;
    renderHwDeviceMenu();
    menu.hidden = false;
    if (btn) btn.setAttribute('aria-expanded', 'true');
    // Focus the previously-selected device (or first) so Enter picks it.
    const sel = menu.querySelector('.hw-device-item.selected')
        || menu.querySelector('.hw-device-item');
    if (sel) sel.focus();
}

function closeHwMenu() {
    const menu = document.getElementById('hw-device-menu');
    const btn = document.getElementById('audio-mode-hw');
    if (menu) menu.hidden = true;
    if (btn) btn.setAttribute('aria-expanded', 'false');
}

function toggleHwMenu() {
    const menu = document.getElementById('hw-device-menu');
    if (!menu) return;
    if (menu.hidden) openHwMenu();
    else closeHwMenu();
}

// Close the menu when clicking outside.
document.addEventListener('click', (e) => {
    const menu = document.getElementById('hw-device-menu');
    if (!menu || menu.hidden) return;
    if (e.target.closest('#hw-device-menu') || e.target.closest('#audio-mode-hw')) return;
    closeHwMenu();
});
document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
        const menu = document.getElementById('hw-device-menu');
        if (menu && !menu.hidden) { closeHwMenu(); e.stopPropagation(); }
    }
});

function handleHwDeviceList(json) {
    try {
        hwDevices = JSON.parse(json) || [];
        const hwBtn = document.getElementById('audio-mode-hw');
        // Clear stale saved selection if that device is gone.
        if (selectedHwDeviceId && !hwDevices.find(d => d.id === selectedHwDeviceId)) {
            selectedHwDeviceId = null;
            try { localStorage.removeItem('lv.hwDevice'); } catch (_) {}
        }
        if (hwDevices.length === 0) {
            if (hwBtn) hwBtn.disabled = true;
            setHwButtonLabel('HW Input');
        } else {
            if (hwBtn) hwBtn.disabled = false;
            const match = selectedHwDeviceId && hwDevices.find(d => d.id === selectedHwDeviceId);
            setHwButtonLabel(match ? (match.name || 'HW Input') : 'HW Input');
        }
        const menu = document.getElementById('hw-device-menu');
        if (menu && !menu.hidden) renderHwDeviceMenu();
    } catch (e) {
        console.error('Failed to parse device list:', e);
    }
}

// Unified audio source picker: one of 'mic' | 'tab' | 'hw' is active at a time.
let currentAudioMode = null;
function setAudioMode(mode) {
    currentAudioMode = mode;
    ['mic', 'tab', 'hw'].forEach(m => {
        const btn = document.getElementById('audio-mode-' + m);
        if (btn) btn.classList.toggle('active', m === mode);
    });
    const stopBtn = document.getElementById('audio-stop-btn');
    if (stopBtn) stopBtn.disabled = !mode;
}

// Handle hardware audio status messages
function handleHwStatus(status) {
    if (status === 'started') {
        hwAudioActive = true;
        setAudioMode('hw');
    } else if (status === 'stopped') {
        hwAudioActive = false;
        if (currentAudioMode === 'hw') setAudioMode(null);
    } else if (status.startsWith('error:')) {
        hwAudioActive = false;
        if (currentAudioMode === 'hw') setAudioMode(null);
        console.error('Hardware audio error:', status.slice(6));
    }
}

function startHwAudio() {
    if (!selectedHwDeviceId) {
        // No device chosen yet — open the picker.
        openHwMenu();
        return;
    }
    stopAudio(); // stop browser-side capture if any
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('hw_audio:start:' + selectedHwDeviceId);
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

// Top-level "switch audio source" — encapsulates stopping the previous mode.
function switchAudioSource(mode) {
    // Stop whatever is currently running.
    if (hwAudioActive) stopHwAudio();
    if (currentAudioMode === 'mic' || currentAudioMode === 'tab') stopAudio();

    if (!mode) { setAudioMode(null); return; }

    if (mode === 'mic') {
        setAudioMode('mic');
        startAudio('mic');
    } else if (mode === 'tab') {
        setAudioMode('tab');
        startAudio('system'); // legacy 'system' mode = getDisplayMedia
    } else if (mode === 'hw') {
        setAudioMode('hw');
        startHwAudio();
    }
}

function stopAudioSource() {
    if (hwAudioActive) stopHwAudio();
    stopAudio();
    setAudioMode(null);
}

// Wire the unified audio picker. HW is special: clicking opens a device menu
// rather than activating immediately; picking a device from the menu activates.
document.querySelectorAll('.audio-mode-btn').forEach(btn => {
    btn.addEventListener('click', (e) => {
        if (btn.dataset.mode === 'hw') {
            e.stopPropagation();
            toggleHwMenu();
        } else {
            switchAudioSource(btn.dataset.mode);
        }
    });
});
const audioStopBtn = document.getElementById('audio-stop-btn');
if (audioStopBtn) audioStopBtn.addEventListener('click', stopAudioSource);

// Bluetooth button listeners
document.getElementById('bt-scan-btn').addEventListener('click', btScan);
document.getElementById('bt-refresh-btn').addEventListener('click', btRefresh);

// Route from current URL path, then connect WebSocket
routeFromPath();
connect();
