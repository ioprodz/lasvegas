// Chromatic circle visualization — shows notes and transitions
const circleCanvas = document.getElementById('chromatic-circle');
const circleCtx = circleCanvas.getContext('2d');

// Note colors (hue-mapped around the circle)
const NOTE_COLORS = [
    '#ff4444', '#ff7744', '#ffaa00', '#dddd00',
    '#44dd00', '#00cc66', '#00bbbb', '#0088ff',
    '#4444ff', '#7744ff', '#aa00ff', '#ff44aa'
];

// Transition history: array of { from, to, time, strength }
const transitions = [];
const MAX_TRANSITIONS = 40;
const TRANSITION_LIFETIME = 4000; // ms

// Note activity: per-note glow intensity (0-1)
const noteActivity = new Float32Array(12);
const NOTE_DECAY = 0.92;

// Chord note highlights
let chordNotes = new Set();

let lastNoteIndex = -1;
let lastTransitionTime = 0;

function updateChromaticCircle() {
    const now = performance.now();

    // Determine current note index
    let currentNoteIdx = -1;
    if (detectedFreq > 20) {
        const semitone = 12 * Math.log2(detectedFreq / 440) + 69;
        currentNoteIdx = ((Math.round(semitone) % 12) + 12) % 12;
    }

    // Decay all notes
    for (let i = 0; i < 12; i++) {
        noteActivity[i] *= NOTE_DECAY;
    }

    // Activate current note
    if (currentNoteIdx >= 0) {
        noteActivity[currentNoteIdx] = 1;

        // Record transition
        if (lastNoteIndex >= 0 && lastNoteIndex !== currentNoteIdx && (now - lastTransitionTime) > 80) {
            transitions.push({
                from: lastNoteIndex,
                to: currentNoteIdx,
                time: now,
                strength: 1
            });
            if (transitions.length > MAX_TRANSITIONS) transitions.shift();
            lastTransitionTime = now;
        }
        lastNoteIndex = currentNoteIdx;
    }

    // Parse chord notes
    chordNotes.clear();
    if (detectedChord !== '\u2014' && currentNoteIdx >= 0) {
        // Root is current strongest note
        chordNotes.add(currentNoteIdx);
        const chord = detectedChord.toLowerCase();
        if (chord.includes('maj7')) {
            chordNotes.add((currentNoteIdx + 4) % 12);
            chordNotes.add((currentNoteIdx + 7) % 12);
            chordNotes.add((currentNoteIdx + 11) % 12);
        } else if (chord.includes('m7')) {
            chordNotes.add((currentNoteIdx + 3) % 12);
            chordNotes.add((currentNoteIdx + 7) % 12);
            chordNotes.add((currentNoteIdx + 10) % 12);
        } else if (chord.includes('dim')) {
            chordNotes.add((currentNoteIdx + 3) % 12);
            chordNotes.add((currentNoteIdx + 6) % 12);
        } else if (chord.includes('aug')) {
            chordNotes.add((currentNoteIdx + 4) % 12);
            chordNotes.add((currentNoteIdx + 8) % 12);
        } else if (chord.includes('sus4')) {
            chordNotes.add((currentNoteIdx + 5) % 12);
            chordNotes.add((currentNoteIdx + 7) % 12);
        } else if (chord.includes('sus2')) {
            chordNotes.add((currentNoteIdx + 2) % 12);
            chordNotes.add((currentNoteIdx + 7) % 12);
        } else if (chord.includes('7')) {
            chordNotes.add((currentNoteIdx + 4) % 12);
            chordNotes.add((currentNoteIdx + 7) % 12);
            chordNotes.add((currentNoteIdx + 10) % 12);
        } else if (chord.includes('m')) {
            chordNotes.add((currentNoteIdx + 3) % 12);
            chordNotes.add((currentNoteIdx + 7) % 12);
        } else if (chord.includes('maj')) {
            chordNotes.add((currentNoteIdx + 4) % 12);
            chordNotes.add((currentNoteIdx + 7) % 12);
        } else if (chord.includes('5')) {
            chordNotes.add((currentNoteIdx + 7) % 12);
        }
    }

    drawChromaticCircle(now);
}

function drawChromaticCircle(now) {
    const rect = circleCanvas.getBoundingClientRect();
    const dpr = devicePixelRatio;
    circleCanvas.width = rect.width * dpr;
    circleCanvas.height = rect.height * dpr;
    circleCtx.scale(dpr, dpr);
    const w = rect.width;
    const h = rect.height;
    circleCtx.clearRect(0, 0, w, h);

    const cx = w / 2;
    const cy = h / 2;
    const radius = Math.min(cx, cy) - 24;
    const noteRadius = 12;

    // Draw transition arrows (oldest first so newest are on top)
    for (let t = transitions.length - 1; t >= 0; t--) {
        const tr = transitions[t];
        const age = now - tr.time;
        if (age > TRANSITION_LIFETIME) {
            transitions.splice(t, 1);
            continue;
        }
        const alpha = (1 - age / TRANSITION_LIFETIME) * 0.6;

        const fromAngle = (tr.from / 12) * Math.PI * 2 - Math.PI / 2;
        const toAngle = (tr.to / 12) * Math.PI * 2 - Math.PI / 2;
        const innerR = radius - noteRadius - 4;

        const x1 = cx + Math.cos(fromAngle) * innerR;
        const y1 = cy + Math.sin(fromAngle) * innerR;
        const x2 = cx + Math.cos(toAngle) * innerR;
        const y2 = cy + Math.sin(toAngle) * innerR;

        // Curved arrow using quadratic bezier through center offset
        const mx = (x1 + x2) / 2;
        const my = (y1 + y2) / 2;
        const dx = x2 - x1;
        const dy = y2 - y1;
        const dist = Math.sqrt(dx * dx + dy * dy);
        // Perpendicular offset toward center for curve
        const nx = -dy / (dist || 1);
        const ny = dx / (dist || 1);
        const curveFactor = dist * 0.25;
        // Bias toward center
        const toCenterX = cx - mx;
        const toCenterY = cy - my;
        const toCenterDist = Math.sqrt(toCenterX * toCenterX + toCenterY * toCenterY) || 1;
        const cpx = mx + (toCenterX / toCenterDist) * curveFactor;
        const cpy = my + (toCenterY / toCenterDist) * curveFactor;

        circleCtx.beginPath();
        circleCtx.moveTo(x1, y1);
        circleCtx.quadraticCurveTo(cpx, cpy, x2, y2);
        circleCtx.strokeStyle = NOTE_COLORS[tr.from];
        circleCtx.globalAlpha = alpha;
        circleCtx.lineWidth = 1.5 + alpha;
        circleCtx.stroke();

        // Arrowhead
        const t2 = 0.92;
        const ax = (1-t2)*(1-t2)*x1 + 2*(1-t2)*t2*cpx + t2*t2*x2;
        const ay = (1-t2)*(1-t2)*y1 + 2*(1-t2)*t2*cpy + t2*t2*y2;
        const angle = Math.atan2(y2 - ay, x2 - ax);
        const headLen = 6;
        circleCtx.beginPath();
        circleCtx.moveTo(x2, y2);
        circleCtx.lineTo(x2 - headLen * Math.cos(angle - 0.4), y2 - headLen * Math.sin(angle - 0.4));
        circleCtx.moveTo(x2, y2);
        circleCtx.lineTo(x2 - headLen * Math.cos(angle + 0.4), y2 - headLen * Math.sin(angle + 0.4));
        circleCtx.stroke();
    }
    circleCtx.globalAlpha = 1;

    // Draw chord connections (inner polygon)
    if (chordNotes.size >= 2) {
        const chordArr = [...chordNotes];
        circleCtx.beginPath();
        for (let i = 0; i < chordArr.length; i++) {
            const angle = (chordArr[i] / 12) * Math.PI * 2 - Math.PI / 2;
            const x = cx + Math.cos(angle) * (radius - noteRadius - 4);
            const y = cy + Math.sin(angle) * (radius - noteRadius - 4);
            if (i === 0) circleCtx.moveTo(x, y);
            else circleCtx.lineTo(x, y);
        }
        circleCtx.closePath();
        circleCtx.fillStyle = 'rgba(255,255,255,0.04)';
        circleCtx.fill();
        circleCtx.strokeStyle = 'rgba(255,255,255,0.2)';
        circleCtx.lineWidth = 1;
        circleCtx.stroke();
    }

    // Draw note nodes
    for (let i = 0; i < 12; i++) {
        const angle = (i / 12) * Math.PI * 2 - Math.PI / 2;
        const x = cx + Math.cos(angle) * radius;
        const y = cy + Math.sin(angle) * radius;
        const activity = noteActivity[i];
        const isChord = chordNotes.has(i);

        // Glow
        if (activity > 0.05) {
            const glowR = noteRadius + 6 + activity * 8;
            const grad = circleCtx.createRadialGradient(x, y, noteRadius * 0.5, x, y, glowR);
            grad.addColorStop(0, NOTE_COLORS[i] + Math.round(activity * 80).toString(16).padStart(2, '0'));
            grad.addColorStop(1, 'transparent');
            circleCtx.fillStyle = grad;
            circleCtx.beginPath();
            circleCtx.arc(x, y, glowR, 0, Math.PI * 2);
            circleCtx.fill();
        }

        // Circle
        const bgAlpha = activity > 0.1 ? 0.9 : (isChord ? 0.5 : 0.15);
        circleCtx.beginPath();
        circleCtx.arc(x, y, noteRadius, 0, Math.PI * 2);
        circleCtx.fillStyle = NOTE_COLORS[i];
        circleCtx.globalAlpha = bgAlpha;
        circleCtx.fill();
        circleCtx.globalAlpha = 1;

        // Border for chord notes
        if (isChord) {
            circleCtx.strokeStyle = '#fff';
            circleCtx.lineWidth = 2;
            circleCtx.stroke();
        }

        // Label
        circleCtx.fillStyle = activity > 0.1 ? '#fff' : '#aaa';
        circleCtx.font = `${activity > 0.1 ? 'bold ' : ''}10px system-ui`;
        circleCtx.textAlign = 'center';
        circleCtx.textBaseline = 'middle';
        circleCtx.fillText(NOTE_NAMES[i], x, y);
    }
}
