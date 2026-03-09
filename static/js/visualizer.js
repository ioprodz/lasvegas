// Frequency visualizer, instrument detection, BPM tracking
const bandLabels = ['Sub', 'Bass', 'Low', 'Mid', 'UMid', 'Pres', 'Bril', 'Treb'];
const bandColors = [
    '#ff0000', '#ff4400', '#ff8800', '#44ff00',
    '#00ffaa', '#0088ff', '#4400ff', '#aa00ff'
];

const prevBands = new Float32Array(8);
let prevBassVal = 0;
let prevMidVal = 0;
let prevHighVal = 0;
const bassOnsets = [];
let lastOnsetTime = 0;
let estimatedBPM = 0;
let beatPhase = 0;
let beatInterval = 0;
let indKick = 0, indSnare = 0, indHihat = 0;
let indVocals = 0, indBassLine = 0;
let spectralFlux = 0;

function drawVisualizer(bands) {
    const canvas = document.getElementById('freq-canvas');
    const ctx = canvas.getContext('2d');
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * devicePixelRatio;
    canvas.height = rect.height * devicePixelRatio;
    ctx.scale(devicePixelRatio, devicePixelRatio);
    const w = rect.width;
    const h = rect.height;
    ctx.clearRect(0, 0, w, h);

    const barW = (w - 20) / 8;
    const gap = 3;

    for (let i = 0; i < 8; i++) {
        const x = 10 + i * barW;
        const barH = (bands[i] / 255) * (h - 22);
        ctx.fillStyle = bandColors[i];
        ctx.globalAlpha = 0.85;
        ctx.beginPath();
        ctx.roundRect(x + gap/2, h - 18 - barH, barW - gap, barH, 2);
        ctx.fill();
        ctx.globalAlpha = 1;
        ctx.fillStyle = '#555';
        ctx.font = '9px system-ui';
        ctx.textAlign = 'center';
        ctx.fillText(bandLabels[i], x + barW/2, h - 4);
    }

    // Spectral analysis
    const now = performance.now();
    const total = bands.reduce((a, b) => a + b, 0) || 1;
    const norm = Array.from(bands).map(b => b / total);
    const centroid = norm.reduce((sum, n, i) => sum + i * n, 0);
    const variance = norm.reduce((sum, n, i) => {
        const d = i - centroid;
        return sum + d * d * n;
    }, 0);
    const spread = Math.sqrt(variance);
    const bassRatio = (norm[0] + norm[1]) * 100;
    const midRatio = (norm[2] + norm[3] + norm[4]) * 100;
    const highRatio = (norm[5] + norm[6] + norm[7]) * 100;
    const peak = Math.max(...bands);
    const dominant = bandLabels[bands.indexOf(peak)];

    // Spectral flux
    let flux = 0;
    for (let i = 0; i < 8; i++) {
        const d = bands[i] - prevBands[i];
        if (d > 0) flux += d;
        prevBands[i] = bands[i];
    }
    spectralFlux = spectralFlux * 0.7 + flux * 0.3;

    // Instrument detection
    const bassVal = (bands[0] + bands[1]) / 2;
    const midVal = (bands[2] + bands[3] + bands[4]) / 3;
    const highVal = (bands[5] + bands[6] + bands[7]) / 3;

    const bassDelta = Math.max(0, bassVal - prevBassVal);
    const kickRaw = bassDelta > 25 ? Math.min(1, bassDelta / 80) : 0;
    indKick = indKick * 0.6 + kickRaw * 0.4;

    const midDelta = Math.max(0, midVal - prevMidVal);
    const highDelta = Math.max(0, highVal - prevHighVal);
    const snareRaw = (midDelta + highDelta) > 40 ? Math.min(1, (midDelta + highDelta) / 120) : 0;
    indSnare = indSnare * 0.6 + snareRaw * 0.4;

    const hihatRaw = Math.min(1, (bands[6] + bands[7]) / 300);
    indHihat = indHihat * 0.8 + hihatRaw * 0.2;

    const vocalZone = centroid > 1.5 && centroid < 5.5 && spread < 2.2;
    const midSustain = midVal > 60;
    const vocalRaw = (vocalZone && midSustain) ? Math.min(1, midVal / 180) : 0;
    indVocals = indVocals * 0.9 + vocalRaw * 0.1;

    const bassLineSustain = bassVal > 80 && bassDelta < 20;
    const bassLineRaw = bassLineSustain ? Math.min(1, bassVal / 200) : 0;
    indBassLine = indBassLine * 0.9 + bassLineRaw * 0.1;

    prevBassVal = bassVal;
    prevMidVal = midVal;
    prevHighVal = highVal;

    // BPM estimation
    if (kickRaw > 0.3 && (now - lastOnsetTime) > 200) {
        bassOnsets.push(now);
        lastOnsetTime = now;
        while (bassOnsets.length > 20) bassOnsets.shift();

        if (bassOnsets.length >= 4) {
            const intervals = [];
            for (let i = 1; i < bassOnsets.length; i++) {
                intervals.push(bassOnsets[i] - bassOnsets[i - 1]);
            }
            intervals.sort((a, b) => a - b);
            const median = intervals[Math.floor(intervals.length / 2)];
            beatInterval = median;
            estimatedBPM = 60000 / median;
            if (estimatedBPM > 200) estimatedBPM /= 2;
            if (estimatedBPM < 50) estimatedBPM *= 2;
        }
    }

    if (beatInterval > 0) {
        beatPhase = ((now - lastOnsetTime) % beatInterval) / beatInterval;
    }

    // Render stats
    const bar = (val, color) => {
        const w = Math.round(val * 100);
        return `<span style="display:inline-block;width:${w}%;max-width:80px;height:6px;background:${color};border-radius:2px;vertical-align:middle;"></span>`;
    };

    const el = document.getElementById('freq-stats');
    el.innerHTML =
        `<span style="color:#888;font-size:0.65rem;text-transform:uppercase;letter-spacing:0.06em">Spectrum</span><br>` +
        `<span style="color:#e0e0e0">Centroid</span> ${centroid.toFixed(2)}` +
        ` <span style="color:#e0e0e0">Spread</span> ${spread.toFixed(2)}<br>` +
        `<span style="color:#e0e0e0">Dominant</span> ${dominant}` +
        ` <span style="color:#e0e0e0">Flux</span> ${Math.round(spectralFlux)}<br>` +
        `<span style="color:#ff6666">Bass</span> ${bassRatio.toFixed(0)}%` +
        ` <span style="color:#44ff88">Mid</span> ${midRatio.toFixed(0)}%` +
        ` <span style="color:#6688ff">High</span> ${highRatio.toFixed(0)}%<br>` +
        `<br>` +
        `<span style="color:#888;font-size:0.65rem;text-transform:uppercase;letter-spacing:0.06em">Tempo</span><br>` +
        `<span style="color:#e0e0e0">BPM</span> ${estimatedBPM > 0 ? Math.round(estimatedBPM) : '\u2014'}` +
        ` <span style="color:#e0e0e0">Phase</span> ${beatPhase.toFixed(2)}<br>` +
        `<br>` +
        `<span style="color:#888;font-size:0.65rem;text-transform:uppercase;letter-spacing:0.06em">Pitch</span><br>` +
        `<span style="color:#fff;font-size:1.1rem;font-weight:600">${detectedNote}</span>` +
        ` <span style="color:#888">${detectedFreq > 0 ? Math.round(detectedFreq) + 'Hz' : ''}</span><br>` +
        `<span style="color:#e0e0e0">Chord</span> <span style="color:#ffcc44;font-weight:500">${detectedChord}</span><br>` +
        `<br>` +
        `<span style="color:#888;font-size:0.65rem;text-transform:uppercase;letter-spacing:0.06em">Instruments</span><br>` +
        `<span style="color:#ff4444">Kick</span>  ${bar(indKick, '#ff4444')}<br>` +
        `<span style="color:#ffaa44">Snare</span> ${bar(indSnare, '#ffaa44')}<br>` +
        `<span style="color:#ffff66">HiHat</span> ${bar(indHihat, '#ffff66')}<br>` +
        `<span style="color:#44ddff">Vocal</span> ${bar(indVocals, '#44ddff')}<br>` +
        `<span style="color:#ff66aa">Bass</span>  ${bar(indBassLine, '#ff66aa')}`;
}
