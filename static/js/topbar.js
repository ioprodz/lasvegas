// Topbar: running-animation chip, stop button visibility, connection LED.

const runningChip = document.getElementById('running-chip');
const runningChipText = document.getElementById('running-chip-text');
const connLed = document.getElementById('conn-led');
const connText = document.getElementById('conn-text');
const helpBtn = document.getElementById('help-btn');

function updateRunningChip(label) {
    if (!runningChip || !runningChipText) return;
    if (label) {
        runningChipText.textContent = 'Running: ' + label;
        runningChip.hidden = false;
        runningChip.classList.add('running');
    } else {
        runningChip.hidden = true;
        runningChip.classList.remove('running');
    }
}

function setConnectionStatus(connected) {
    if (!connLed || !connText) return;
    connLed.classList.toggle('on', connected);
    connText.textContent = connected ? 'Connected' : 'Offline';
}

if (helpBtn) {
    helpBtn.addEventListener('click', () => {
        if (typeof toggleCheatsheet === 'function') toggleCheatsheet();
    });
}

// Signal Viz/Stats tabs
const signalTabs = document.getElementById('signal-tabs');
if (signalTabs) {
    signalTabs.addEventListener('click', (e) => {
        const btn = e.target.closest('button[data-tab]');
        if (!btn) return;
        const which = btn.dataset.tab;
        signalTabs.querySelectorAll('button').forEach(b =>
            b.classList.toggle('active', b === btn));
        document.querySelectorAll('.signal-pane').forEach(p =>
            p.hidden = (p.dataset.pane !== which));
    });
}

// Initial state
updateRunningChip(null);
setConnectionStatus(false);
