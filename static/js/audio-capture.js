// Audio capture, AGC, and WebSocket streaming
let audioCtx = null;
let analyser = null;
let audioSource = null;
let audioStream = null;

// Web Worker timer -- not throttled when tab is backgrounded
const timerWorkerBlob = new Blob([`
    let iv = null;
    self.onmessage = (e) => {
        if (e.data === 'start') { iv = setInterval(() => self.postMessage(0), 33); }
        else if (e.data === 'stop') { clearInterval(iv); iv = null; }
    };
`], { type: 'application/javascript' });
const timerWorker = new Worker(URL.createObjectURL(timerWorkerBlob));

const BAND_RANGES = [
    [0, 2], [2, 6], [6, 12], [12, 23],
    [23, 46], [46, 93], [93, 139], [139, 232],
];

let agcPeak = 30;
const AGC_ATTACK = 0.6;
const AGC_DECAY = 0.002;
const AGC_FLOOR = 8;

function computeBands(freqData) {
    const raw = new Float32Array(8);
    let maxRaw = 0;
    for (let b = 0; b < 8; b++) {
        const [start, end] = BAND_RANGES[b];
        let sum = 0;
        for (let i = start; i < end && i < freqData.length; i++) sum += freqData[i];
        raw[b] = sum / (end - start);
        if (raw[b] > maxRaw) maxRaw = raw[b];
    }

    if (maxRaw > agcPeak) {
        agcPeak += (maxRaw - agcPeak) * AGC_ATTACK;
    } else {
        agcPeak *= (1 - AGC_DECAY);
    }
    agcPeak = Math.max(agcPeak, AGC_FLOOR);

    const bands = new Uint8Array(8);
    const gain = 255 / agcPeak;
    for (let b = 0; b < 8; b++) {
        bands[b] = Math.min(255, Math.round(raw[b] * gain));
    }
    return bands;
}

async function startAudio(mode) {
    stopAudio();
    try {
        if (mode === 'mic') {
            audioStream = await navigator.mediaDevices.getUserMedia({ audio: true });
        } else {
            audioStream = await navigator.mediaDevices.getDisplayMedia({
                audio: true,
                video: { width: 1, height: 1 }
            });
            audioStream.getVideoTracks().forEach(t => t.stop());
        }

        audioCtx = new AudioContext();
        analyser = audioCtx.createAnalyser();
        analyser.fftSize = 512;
        analyser.smoothingTimeConstant = 0.8;

        pitchAnalyser = audioCtx.createAnalyser();
        pitchAnalyser.fftSize = 4096;
        pitchAnalyser.smoothingTimeConstant = 0.85;

        audioSource = audioCtx.createMediaStreamSource(audioStream);
        audioSource.connect(analyser);
        audioSource.connect(pitchAnalyser);

        const freqData = new Uint8Array(analyser.frequencyBinCount);

        timerWorker.onmessage = () => {
            if (!analyser) return;
            analyser.getByteFrequencyData(freqData);
            const bands = computeBands(freqData);
            updatePitchDetection();
            if (document.visibilityState === 'visible') {
                drawVisualizer(bands);
                updateChromaticCircle();
            }
            if (ws && ws.readyState === WebSocket.OPEN) {
                const bpm = Math.round(estimatedBPM) || 0;
                let chordQual = 255;
                if (detectedChord !== '\u2014') {
                    const q = detectedChord.toLowerCase();
                    if (q.includes('maj7')) chordQual = 5;
                    else if (q.includes('m7')) chordQual = 6;
                    else if (q.includes('dim')) chordQual = 2;
                    else if (q.includes('aug')) chordQual = 3;
                    else if (q.includes('sus2')) chordQual = 7;
                    else if (q.includes('sus4')) chordQual = 8;
                    else if (q.includes('7')) chordQual = 4;
                    else if (q.includes('5')) chordQual = 9;
                    else if (q.includes('m')) chordQual = 1;
                    else if (q.includes('maj')) chordQual = 0;
                }
                let parsedRoot = 255;
                if (detectedChord !== '\u2014') {
                    const rootStr = detectedChord.match(/^[A-G]#?/);
                    if (rootStr) {
                        const idx = NOTE_NAMES.indexOf(rootStr[0]);
                        if (idx >= 0) parsedRoot = idx;
                    }
                }
                const midiNote = detectedFreq > 0 ? Math.round(12 * Math.log2(detectedFreq / 440) + 69) : 0;

                const msg = new Uint8Array(20);
                msg[0] = 0x03;
                for (let i = 0; i < 8; i++) msg[1 + i] = bands[i];
                msg[9]  = Math.min(255, Math.round(indKick * 255));
                msg[10] = Math.min(255, Math.round(indSnare * 255));
                msg[11] = Math.min(255, Math.round(indHihat * 255));
                msg[12] = Math.min(255, Math.round(indVocals * 255));
                msg[13] = Math.min(255, Math.round(indBassLine * 255));
                msg[14] = (bpm >> 8) & 0xFF;
                msg[15] = bpm & 0xFF;
                msg[16] = Math.min(255, Math.round(beatPhase * 255));
                msg[17] = Math.max(0, Math.min(127, midiNote));
                msg[18] = parsedRoot;
                msg[19] = chordQual;
                ws.send(msg.buffer);
            }
        };
        timerWorker.postMessage('start');

        document.getElementById('btn-mic').classList.toggle('active', mode === 'mic');
        document.getElementById('btn-system').classList.toggle('active', mode === 'system');
        document.getElementById('btn-stop-audio').disabled = false;
    } catch (err) {
        console.error('Audio capture failed:', err);
    }
}

function stopAudio() {
    timerWorker.postMessage('stop');
    if (audioSource) audioSource.disconnect();
    if (audioStream) audioStream.getTracks().forEach(t => t.stop());
    if (audioCtx) audioCtx.close();
    audioCtx = null; analyser = null; pitchAnalyser = null; audioSource = null; audioStream = null;
    document.getElementById('btn-mic').classList.remove('active');
    document.getElementById('btn-system').classList.remove('active');
    document.getElementById('btn-stop-audio').disabled = true;
    const canvas = document.getElementById('freq-canvas');
    canvas.getContext('2d').clearRect(0, 0, canvas.width, canvas.height);
}
