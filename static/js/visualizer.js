// Frequency visualizer, instrument detection, BPM tracking.
// Band colors are pulled from CSS custom properties so the palette stays
// in sync with the theme.
const bandLabels = ['Sub', 'Bass', 'Low', 'Mid', 'UMid', 'Pres', 'Bril', 'Treb'];

function getCssColor(name, fallback) {
    const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return v || fallback;
}

let bandColors = [];
function refreshBandColors() {
    // Map low→high across magenta→amber→cyan, with red at the bass end.
    const red = getCssColor('--neon-red', '#ff4d6d');
    const amber = getCssColor('--neon-amber', '#ffcc33');
    const green = getCssColor('--neon-green', '#34d399');
    const cyan = getCssColor('--neon-cyan', '#00e6ff');
    const violet = getCssColor('--neon-violet', '#b56bff');
    const magenta = getCssColor('--neon-magenta', '#ff2bd6');
    bandColors = [red, '#ff7a3d', amber, green, '#00d3aa', cyan, violet, magenta];
}
refreshBandColors();

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

let _freqCanvasResizer = null;
function ensureCanvasObserver() {
    if (_freqCanvasResizer) return;
    const c = document.getElementById('freq-canvas');
    if (!c) return;
    _freqCanvasResizer = new ResizeObserver(() => { /* canvas re-resizes inside drawVisualizer */ });
    _freqCanvasResizer.observe(c);
}

function drawVisualizer(bands) {
    const canvas = document.getElementById('freq-canvas');
    if (!canvas) return;
    ensureCanvasObserver();
    const ctx = canvas.getContext('2d');
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== Math.round(rect.width * dpr) || canvas.height !== Math.round(rect.height * dpr)) {
        canvas.width = Math.round(rect.width * dpr);
        canvas.height = Math.round(rect.height * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const w = rect.width;
    const h = rect.height;
    ctx.clearRect(0, 0, w, h);

    const barW = (w - 16) / 8;
    const gap = 3;

    for (let i = 0; i < 8; i++) {
        const x = 8 + i * barW;
        const barH = (bands[i] / 255) * (h - 22);
        ctx.fillStyle = bandColors[i];
        ctx.globalAlpha = 0.85;
        ctx.beginPath();
        if (ctx.roundRect) ctx.roundRect(x + gap/2, h - 18 - barH, barW - gap, barH, 2);
        else ctx.rect(x + gap/2, h - 18 - barH, barW - gap, barH);
        ctx.fill();
        ctx.globalAlpha = 1;
        ctx.fillStyle = getCssColor('--text-dim', '#5a5a78');
        ctx.font = '9px ui-sans-serif, system-ui, sans-serif';
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

    // Stats panel
    const bar = (val, color) => {
        const w = Math.round(val * 100);
        return `<span style="display:inline-block;width:${w}%;max-width:80px;height:6px;background:${color};border-radius:2px;vertical-align:middle;box-shadow:0 0 4px ${color}55"></span>`;
    };

    const el = document.getElementById('freq-stats');
    if (!el) return;
    const label = getCssColor('--label', '#7a7a98');
    const text = getCssColor('--text', '#f5f5fa');
    const cyan = getCssColor('--neon-cyan', '#00e6ff');
    const amber = getCssColor('--neon-amber', '#ffcc33');
    const red = getCssColor('--neon-red', '#ff4d6d');
    const green = getCssColor('--neon-green', '#34d399');
    const magenta = getCssColor('--neon-magenta', '#ff2bd6');
    el.innerHTML =
        `<span style="color:${label};font-size:0.65rem;text-transform:uppercase;letter-spacing:0.1em">Spectrum</span><br>` +
        `<span style="color:${text}">Centroid</span> ${centroid.toFixed(2)}` +
        ` <span style="color:${text}">Spread</span> ${spread.toFixed(2)}<br>` +
        `<span style="color:${text}">Dominant</span> ${dominant}` +
        ` <span style="color:${text}">Flux</span> ${Math.round(spectralFlux)}<br>` +
        `<span style="color:${red}">Bass</span> ${bassRatio.toFixed(0)}%` +
        ` <span style="color:${green}">Mid</span> ${midRatio.toFixed(0)}%` +
        ` <span style="color:${cyan}">High</span> ${highRatio.toFixed(0)}%<br>` +
        `<br>` +
        `<span style="color:${label};font-size:0.65rem;text-transform:uppercase;letter-spacing:0.1em">Tempo</span><br>` +
        `<span style="color:${text}">BPM</span> ${estimatedBPM > 0 ? Math.round(estimatedBPM) : '—'}` +
        ` <span style="color:${text}">Phase</span> ${beatPhase.toFixed(2)}<br>` +
        `<br>` +
        `<span style="color:${label};font-size:0.65rem;text-transform:uppercase;letter-spacing:0.1em">Pitch</span><br>` +
        `<span style="color:${cyan};font-size:1.1rem;font-weight:600;text-shadow:0 0 8px ${cyan}55">${detectedNote}</span>` +
        ` <span style="color:${label}">${detectedFreq > 0 ? Math.round(detectedFreq) + 'Hz' : ''}</span><br>` +
        `<span style="color:${text}">Chord</span> <span style="color:${amber};font-weight:600;text-shadow:0 0 6px ${amber}55">${detectedChord}</span><br>` +
        `<br>` +
        `<span style="color:${label};font-size:0.65rem;text-transform:uppercase;letter-spacing:0.1em">Instruments</span><br>` +
        `<span style="color:${red}">Kick</span>  ${bar(indKick, red)}<br>` +
        `<span style="color:${amber}">Snare</span> ${bar(indSnare, amber)}<br>` +
        `<span style="color:#ffee66">HiHat</span> ${bar(indHihat, '#ffee66')}<br>` +
        `<span style="color:${cyan}">Vocal</span> ${bar(indVocals, cyan)}<br>` +
        `<span style="color:${magenta}">Bass</span>  ${bar(indBassLine, magenta)}`;
}
