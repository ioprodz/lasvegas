// Keyboard shortcuts for the Performance page.
// 1–9 activate shortcut slots. Arrows navigate the 3x3 slot grid.
// Esc/Space stop. M/B/H switch audio source. ? toggles the cheatsheet.

(function () {
    function isEditableTarget(el) {
        if (!el) return false;
        const tag = el.tagName;
        return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
    }

    function onPerformance() {
        return document.body.dataset.page === 'performance';
    }

    function focusedSlotIdx() {
        const slots = Array.from(document.querySelectorAll('.shortcut-slot'));
        const i = slots.indexOf(document.activeElement);
        if (i >= 0) return i;
        // also check if a child of a slot is focused (e.g. the card)
        const closest = document.activeElement && document.activeElement.closest
            ? document.activeElement.closest('.shortcut-slot')
            : null;
        return closest ? slots.indexOf(closest) : -1;
    }

    function focusSlot(idx) {
        const slots = document.querySelectorAll('.shortcut-slot');
        const slot = slots[idx];
        if (!slot) return;
        const card = slot.querySelector('.anim-card');
        if (card) {
            card.tabIndex = 0;
            card.focus();
        } else {
            slot.tabIndex = 0;
            slot.focus();
        }
    }

    function moveFocus(dir) {
        const idx = focusedSlotIdx();
        const start = idx < 0 ? 0 : idx;
        const row = Math.floor(start / 3);
        const col = start % 3;
        let nr = row, nc = col;
        if (dir === 'left')  nc = Math.max(0, col - 1);
        if (dir === 'right') nc = Math.min(2, col + 1);
        if (dir === 'up')    nr = Math.max(0, row - 1);
        if (dir === 'down')  nr = Math.min(2, row + 1);
        focusSlot(nr * 3 + nc);
    }

    document.addEventListener('keydown', (e) => {
        // ? toggles cheatsheet anywhere.
        if ((e.key === '?' || (e.key === '/' && e.shiftKey)) && !isEditableTarget(e.target)) {
            e.preventDefault();
            if (typeof toggleCheatsheet === 'function') toggleCheatsheet();
            return;
        }
        if (e.key === 'Escape') {
            const overlay = document.getElementById('cheatsheet-overlay');
            if (overlay && overlay.classList.contains('open')) {
                if (typeof toggleCheatsheet === 'function') toggleCheatsheet(false);
                e.preventDefault();
                return;
            }
            if (onPerformance() && !isEditableTarget(e.target)) {
                const stopBtn = document.getElementById('stop-btn');
                if (stopBtn && !stopBtn.disabled) stopBtn.click();
                e.preventDefault();
                return;
            }
        }
        if (!onPerformance()) return;
        if (isEditableTarget(e.target)) return;
        if (e.metaKey || e.ctrlKey || e.altKey) return;

        if (e.key >= '1' && e.key <= '9') {
            const idx = parseInt(e.key, 10) - 1;
            if (typeof activateSlot === 'function') activateSlot(idx);
            e.preventDefault();
            return;
        }
        if (e.key === 'ArrowLeft')  { moveFocus('left');  e.preventDefault(); return; }
        if (e.key === 'ArrowRight') { moveFocus('right'); e.preventDefault(); return; }
        if (e.key === 'ArrowUp')    { moveFocus('up');    e.preventDefault(); return; }
        if (e.key === 'ArrowDown')  { moveFocus('down');  e.preventDefault(); return; }

        if (e.key === 'Enter') {
            const idx = focusedSlotIdx();
            if (idx >= 0 && typeof activateSlot === 'function') {
                activateSlot(idx);
                e.preventDefault();
                return;
            }
        }

        if (e.key === ' ' || e.code === 'Space') {
            const stopBtn = document.getElementById('stop-btn');
            if (stopBtn && !stopBtn.disabled) {
                stopBtn.click();
                e.preventDefault();
                return;
            }
        }

        // Audio source hotkeys
        const k = e.key.toLowerCase();
        if (k === 'm' || k === 'b' || k === 'h') {
            const id = ({ m: 'audio-mode-mic', b: 'audio-mode-tab', h: 'audio-mode-hw' })[k];
            const btn = document.getElementById(id);
            if (btn && !btn.disabled && getComputedStyle(btn).display !== 'none') {
                btn.click();
                e.preventDefault();
            }
            return;
        }
        if (e.key === 'Tab') {
            // Cycle through enabled audio modes.
            const order = ['audio-mode-mic', 'audio-mode-tab', 'audio-mode-hw'];
            const currentIdx = order.findIndex(id => {
                const el = document.getElementById(id);
                return el && el.classList.contains('active');
            });
            for (let i = 1; i <= order.length; i++) {
                const next = (currentIdx + i + order.length) % order.length;
                const el = document.getElementById(order[next]);
                if (!el || el.disabled) continue;
                const style = getComputedStyle(el);
                if (style.display === 'none') continue;
                el.click();
                e.preventDefault();
                return;
            }
        }
    });

    // Hide kbd badges on touch-only devices.
    function detectTouchOnly() {
        if (window.matchMedia && window.matchMedia('(pointer: coarse)').matches
            && !window.matchMedia('(pointer: fine)').matches) {
            document.body.classList.add('no-kbd');
        }
    }
    detectTouchOnly();
    window.addEventListener('keydown', () => {
        document.body.classList.remove('no-kbd');
    }, { once: true });
})();
