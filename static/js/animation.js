// Animation state. Cards are clickable both in the library list and inside
// shortcut slots. The 9 slot positions are owned by shortcuts.js.

const allAnimCards = document.querySelectorAll('.anim-card[data-anim]');
const stopBtn = document.getElementById('stop-btn');
const ANIM_LABELS = {};
allAnimCards.forEach(card => {
    const h3 = card.querySelector('h3');
    ANIM_LABELS[card.dataset.anim] = h3 ? h3.textContent.trim() : card.dataset.anim;
});

let currentAnim = null;

function setActiveAnim(name) {
    currentAnim = name;
    allAnimCards.forEach(card => {
        card.classList.toggle('running', card.dataset.anim === name);
    });
    if (stopBtn) {
        stopBtn.disabled = !name;
        stopBtn.classList.toggle('visible', !!name);
    }
    if (typeof updateRunningChip === 'function') {
        updateRunningChip(name ? (ANIM_LABELS[name] || name) : null);
    }
}

allAnimCards.forEach(card => {
    card.addEventListener('click', () => {
        const name = card.dataset.anim;
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send('animate:' + name);
            setActiveAnim(name);
        }
    });
});

if (stopBtn) {
    stopBtn.addEventListener('click', () => {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send('stop');
            setActiveAnim(null);
        }
    });
}
