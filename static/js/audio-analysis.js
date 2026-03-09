// Pitch and chord detection
const NOTE_NAMES = ['C','C#','D','D#','E','F','F#','G','G#','A','A#','B'];
let detectedNote = '\u2014';
let detectedChord = '\u2014';
let detectedFreq = 0;
let pitchAnalyser = null;

function freqToNote(freq) {
    if (freq < 20) return { name: '\u2014', octave: 0, cents: 0 };
    const semitone = 12 * Math.log2(freq / 440) + 69;
    const rounded = Math.round(semitone);
    const cents = Math.round((semitone - rounded) * 100);
    const name = NOTE_NAMES[((rounded % 12) + 12) % 12];
    const octave = Math.floor(rounded / 12) - 1;
    return { name, octave, cents, midi: rounded };
}

function findPeaks(freqData, sampleRate, fftSize, maxPeaks) {
    const binHz = sampleRate / fftSize;
    const peaks = [];
    const minBin = Math.ceil(60 / binHz);
    const maxBin = Math.min(freqData.length - 1, Math.floor(4200 / binHz));

    for (let i = minBin + 1; i < maxBin; i++) {
        if (freqData[i] > freqData[i-1] && freqData[i] > freqData[i+1] && freqData[i] > 30) {
            const alpha = freqData[i-1];
            const beta = freqData[i];
            const gamma = freqData[i+1];
            const denom = alpha - 2*beta + gamma;
            const p = denom !== 0 ? 0.5 * (alpha - gamma) / denom : 0;
            const interpFreq = (i + p) * binHz;
            peaks.push({ freq: interpFreq, amp: freqData[i] });
        }
    }

    peaks.sort((a, b) => b.amp - a.amp);
    return peaks.slice(0, maxPeaks);
}

function detectChord(notes) {
    if (notes.length < 2) return '\u2014';
    const root = notes[0];
    const intervals = new Set();
    for (const n of notes) {
        intervals.add(((n.midi - root.midi) % 12 + 12) % 12);
    }
    const has = (i) => intervals.has(i);
    const rootName = root.name;

    if (has(4) && has(7)) {
        if (has(11)) return rootName + 'maj7';
        if (has(10)) return rootName + '7';
        return rootName + ' maj';
    }
    if (has(3) && has(7)) {
        if (has(10)) return rootName + 'm7';
        return rootName + 'm';
    }
    if (has(4) && has(8)) return rootName + ' aug';
    if (has(3) && has(6)) return rootName + ' dim';
    if (has(5) && has(7)) return rootName + 'sus4';
    if (has(2) && has(7)) return rootName + 'sus2';
    if (has(7)) return rootName + '5';
    return rootName + '?';
}

function updatePitchDetection() {
    if (!pitchAnalyser || !audioCtx) return;
    const fftSize = pitchAnalyser.fftSize;
    const freqData = new Uint8Array(pitchAnalyser.frequencyBinCount);
    pitchAnalyser.getByteFrequencyData(freqData);

    const peaks = findPeaks(freqData, audioCtx.sampleRate, fftSize, 6);

    if (peaks.length > 0) {
        detectedFreq = peaks[0].freq;
        const note = freqToNote(peaks[0].freq);
        detectedNote = note.name + note.octave;
        const noteInfos = peaks
            .filter(p => p.amp > peaks[0].amp * 0.3)
            .map(p => ({ ...freqToNote(p.freq), amp: p.amp }));
        detectedChord = detectChord(noteInfos);
    } else {
        detectedNote = '\u2014';
        detectedChord = '\u2014';
        detectedFreq = 0;
    }
}
