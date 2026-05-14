// Persistent LED preview dock. Always present; collapsible via the top-bar toggle.

const gridContainer = document.getElementById('grid');
const ledDock = document.getElementById('led-dock');
const previewToggle = document.getElementById('preview-toggle');
const gridLeds = [];

for (let i = 0; i < 360; i++) {
    const led = document.createElement('div');
    led.className = 'led';
    gridContainer.appendChild(led);
    gridLeds.push(led);
}

// Restore collapsed state.
try {
    if (localStorage.getItem('lv.previewCollapsed') === '1') {
        ledDock.classList.add('collapsed');
    }
} catch (_) {}

if (previewToggle) {
    previewToggle.addEventListener('click', () => {
        const collapsed = ledDock.classList.toggle('collapsed');
        try { localStorage.setItem('lv.previewCollapsed', collapsed ? '1' : '0'); } catch (_) {}
    });
}

function updateGrid(data) {
    if (ledDock.classList.contains('collapsed')) return;
    for (let i = 0; i < 360 && i * 3 + 2 < data.length; i++) {
        gridLeds[i].style.background = `rgb(${data[i*3]},${data[i*3+1]},${data[i*3+2]})`;
    }
}
