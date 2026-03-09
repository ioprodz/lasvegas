// Calibration controls
const calSliders = {
    r: { min: document.getElementById('cal-r-min'), max: document.getElementById('cal-r-max'), gamma: document.getElementById('cal-r-gamma'), fill: document.getElementById('cal-r-fill'), rangeVal: document.getElementById('cal-r-range-val') },
    g: { min: document.getElementById('cal-g-min'), max: document.getElementById('cal-g-max'), gamma: document.getElementById('cal-g-gamma'), fill: document.getElementById('cal-g-fill'), rangeVal: document.getElementById('cal-g-range-val') },
    b: { min: document.getElementById('cal-b-min'), max: document.getElementById('cal-b-max'), gamma: document.getElementById('cal-b-gamma'), fill: document.getElementById('cal-b-fill'), rangeVal: document.getElementById('cal-b-range-val') },
};
const calGammaValues = {
    r: document.getElementById('cal-r-gamma-val'),
    g: document.getElementById('cal-g-gamma-val'),
    b: document.getElementById('cal-b-gamma-val'),
};

function clampDualRange(ch, changed) {
    const s = calSliders[ch];
    let minV = parseInt(s.min.value);
    let maxV = parseInt(s.max.value);
    if (changed === 'min' && minV > maxV) s.min.value = maxV;
    if (changed === 'max' && maxV < minV) s.max.value = minV;
}

function updateDualRangeFill(ch) {
    const s = calSliders[ch];
    const minV = parseInt(s.min.value);
    const maxV = parseInt(s.max.value);
    const left = (minV / 255) * 100;
    const right = (maxV / 255) * 100;
    s.fill.style.left = left + '%';
    s.fill.style.width = (right - left) + '%';
    s.rangeVal.textContent = minV + '-' + maxV;
}

function sendCalibration() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const params = ['r','g','b'].map(ch =>
        `${calSliders[ch].min.value},${calSliders[ch].max.value},${calSliders[ch].gamma.value}`
    ).join(',');
    ws.send('calibrate:' + params);
}

function updateCalDisplay() {
    for (const ch of ['r','g','b']) {
        updateDualRangeFill(ch);
        calGammaValues[ch].textContent = parseFloat(calSliders[ch].gamma.value).toFixed(2);
    }
}

for (const ch of ['r','g','b']) {
    calSliders[ch].min.addEventListener('input', () => {
        clampDualRange(ch, 'min');
        updateCalDisplay();
        sendCalibration();
    });
    calSliders[ch].max.addEventListener('input', () => {
        clampDualRange(ch, 'max');
        updateCalDisplay();
        sendCalibration();
    });
    calSliders[ch].gamma.addEventListener('input', () => {
        updateCalDisplay();
        sendCalibration();
    });
}
updateCalDisplay();

function applyCalibrationFromServer(data) {
    for (const ch of ['r','g','b']) {
        if (data[ch]) {
            calSliders[ch].min.value = data[ch].min;
            calSliders[ch].max.value = data[ch].max;
            calSliders[ch].gamma.value = data[ch].gamma;
        }
    }
    updateCalDisplay();
}

// Test color fill
document.getElementById('cal-fill-btn').addEventListener('click', () => {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const hex = document.getElementById('cal-color').value;
    const brightness = parseInt(document.getElementById('cal-brightness').value) / 255;
    const r = Math.round(parseInt(hex.slice(1,3), 16) * brightness);
    const g = Math.round(parseInt(hex.slice(3,5), 16) * brightness);
    const b = Math.round(parseInt(hex.slice(5,7), 16) * brightness);
    ws.send(new Uint8Array([0x01, r, g, b]).buffer);
});

document.getElementById('cal-off-btn').addEventListener('click', () => {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send('stop');
    setActiveAnim(null);
});

document.querySelectorAll('[data-test-color]').forEach(btn => {
    btn.addEventListener('click', () => {
        if (!ws || ws.readyState !== WebSocket.OPEN) return;
        const [r,g,b] = btn.dataset.testColor.split(',').map(Number);
        ws.send(new Uint8Array([0x01, r, g, b]).buffer);
    });
});

document.getElementById('cal-save-btn').addEventListener('click', () => {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send('save_calibration');
});

document.getElementById('cal-reset-btn').addEventListener('click', () => {
    for (const ch of ['r','g','b']) {
        calSliders[ch].min.value = 0;
        calSliders[ch].max.value = 255;
        calSliders[ch].gamma.value = '1.0';
    }
    updateCalDisplay();
    sendCalibration();
});

// Calibrate nav handler
document.getElementById('nav-calibrate').addEventListener('click', () => {
    showPage('calibrate');
    history.pushState({ page: 'calibrate' }, '', '/calibrate');
    navItems.forEach(n => n.classList.remove('active'));
    document.getElementById('nav-calibrate').classList.add('active');
    sidebar.classList.remove('open');
    if (ws && ws.readyState === WebSocket.OPEN) ws.send('get_calibration');
});
