// Animation state management
const allAnimCards = document.querySelectorAll('.anim-card');
const stopBtn = document.getElementById('stop-btn');
let currentAnim = null;

function setActiveAnim(name) {
    currentAnim = name;
    allAnimCards.forEach(card => {
        card.classList.toggle('running', card.dataset.anim === name);
    });
    navItems.forEach(item => {
        item.classList.toggle('running', item.dataset.anim === name);
        let dot = item.querySelector('.running-dot');
        if (item.dataset.anim === name) {
            if (!dot) {
                dot = document.createElement('span');
                dot.className = 'running-dot';
                item.appendChild(dot);
            }
        } else if (dot) {
            dot.remove();
        }
    });
    stopBtn.disabled = !name;
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

stopBtn.addEventListener('click', () => {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('stop');
        setActiveAnim(null);
    }
});
