// Performance-page shortcuts: 3x3 grid populated by drag-and-drop from the
// animation list. State is mirrored on the server (CSV over WS, persisted to
// shortcuts.conf). Unified pointer-based DnD works on desktop and touch.

(function () {
    const SLOTS = 9;
    const grid = document.getElementById('shortcut-grid');
    const list = document.getElementById('anim-list');
    if (!grid || !list) return;

    const slotEls = Array.from(grid.querySelectorAll('.shortcut-slot'));
    const allCards = Array.from(document.querySelectorAll('.anim-card[data-anim]'));

    // ── State synchronization ────────────────────────────────────────────

    function refreshSlotStates() {
        slotEls.forEach(s => {
            const card = s.querySelector('.anim-card');
            s.classList.toggle('filled', !!card);
        });
    }

    function currentCsv() {
        return slotEls.map(s => {
            const c = s.querySelector('.anim-card');
            return c ? c.dataset.anim : '';
        }).join(',');
    }

    function pushShortcuts() {
        if (typeof ws !== 'undefined' && ws && ws.readyState === WebSocket.OPEN) {
            ws.send('shortcuts:set:' + currentCsv());
        }
    }

    // Apply a server-provided CSV: move cards into slots accordingly.
    // Cards not in any slot end up in the list.
    window.applyShortcuts = function (csv) {
        const want = (csv || '').split(',').slice(0, SLOTS);
        const desired = new Map(); // anim -> slotIdx
        want.forEach((name, i) => {
            const n = (name || '').trim();
            if (n) desired.set(n, i);
        });

        // First pull any card whose slot doesn't match desired position
        allCards.forEach(card => {
            const anim = card.dataset.anim;
            const parent = card.parentElement;
            const inSlotIdx = parent && parent.classList.contains('shortcut-slot')
                ? parseInt(parent.dataset.slot, 10) : -1;
            const wantIdx = desired.has(anim) ? desired.get(anim) : -1;
            if (inSlotIdx !== wantIdx) {
                // remove
                if (parent) parent.removeChild(card);
            }
        });

        // Now place desired cards into their target slots
        desired.forEach((slotIdx, anim) => {
            const card = allCards.find(c => c.dataset.anim === anim);
            if (!card) return;
            const slot = slotEls[slotIdx];
            // Remove any other card that ended up here (defensive)
            const occupant = slot.querySelector('.anim-card');
            if (occupant && occupant !== card) list.appendChild(occupant);
            slot.appendChild(card);
        });

        // Anyone still detached goes back to the list
        allCards.forEach(card => {
            if (!card.parentElement) list.appendChild(card);
        });

        refreshSlotStates();
    };

    // Helper for keyboard.js: activate the animation in slot N (0-indexed).
    window.activateSlot = function (idx) {
        const slot = slotEls[idx];
        if (!slot) return false;
        const card = slot.querySelector('.anim-card');
        if (!card) return false;
        card.click();
        card.focus();
        return true;
    };

    window.getShortcutSlots = function () {
        return slotEls.map(s => s.querySelector('.anim-card'));
    };

    // ── Drag and drop (pointer-based, works on touch and desktop) ────────

    let dragging = null;          // the source card element
    let dragGhost = null;         // floating clone following the pointer
    let dragOffsetX = 0;
    let dragOffsetY = 0;
    let dragStartX = 0;
    let dragStartY = 0;
    let dragStarted = false;
    let suppressNextClick = false;
    const DRAG_THRESHOLD = 6;

    function pointerDown(e) {
        if (e.button !== undefined && e.button !== 0) return;
        const card = e.currentTarget;
        dragging = card;
        dragStarted = false;
        const r = card.getBoundingClientRect();
        dragOffsetX = e.clientX - r.left;
        dragOffsetY = e.clientY - r.top;
        dragStartX = e.clientX;
        dragStartY = e.clientY;
        card.setPointerCapture(e.pointerId);
    }

    function pointerMove(e) {
        if (!dragging) return;
        const dx = e.clientX - dragStartX;
        const dy = e.clientY - dragStartY;
        if (!dragStarted && Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
        if (!dragStarted) {
            dragStarted = true;
            // Build floating ghost
            const r = dragging.getBoundingClientRect();
            dragGhost = dragging.cloneNode(true);
            dragGhost.classList.add('drag-ghost');
            dragGhost.style.position = 'fixed';
            dragGhost.style.left = r.left + 'px';
            dragGhost.style.top = r.top + 'px';
            dragGhost.style.width = r.width + 'px';
            dragGhost.style.pointerEvents = 'none';
            dragGhost.style.zIndex = '1000';
            dragGhost.style.opacity = '0.85';
            dragGhost.style.transform = 'scale(0.96)';
            dragGhost.style.boxShadow = '0 12px 32px rgba(0,0,0,0.6), 0 0 18px var(--neon-magenta-glow)';
            document.body.appendChild(dragGhost);
            dragging.classList.add('dragging');
        }
        if (dragGhost) {
            dragGhost.style.left = (e.clientX - dragOffsetX) + 'px';
            dragGhost.style.top = (e.clientY - dragOffsetY) + 'px';
        }
        // Highlight drop target under pointer
        const target = dropTargetFromPoint(e.clientX, e.clientY);
        document.querySelectorAll('.drop-target').forEach(el => el.classList.remove('drop-target'));
        if (target) target.classList.add('drop-target');
    }

    function pointerUp(e) {
        if (!dragging) return;
        const card = dragging;
        const wasDragging = dragStarted;
        try { card.releasePointerCapture(e.pointerId); } catch (_) {}
        if (wasDragging) {
            const target = dropTargetFromPoint(e.clientX, e.clientY);
            handleDrop(card, target);
            suppressNextClick = true;
            // Click event will fire right after — clear flag after a tick.
            setTimeout(() => { suppressNextClick = false; }, 50);
        }
        // Cleanup
        document.querySelectorAll('.drop-target').forEach(el => el.classList.remove('drop-target'));
        if (dragGhost && dragGhost.parentElement) dragGhost.parentElement.removeChild(dragGhost);
        dragGhost = null;
        card.classList.remove('dragging');
        dragging = null;
        dragStarted = false;
    }

    function pointerCancel(e) {
        pointerUp(e);
    }

    function dropTargetFromPoint(x, y) {
        // Temporarily hide the ghost so elementFromPoint sees the underlying target.
        let prevDisplay = null;
        if (dragGhost) { prevDisplay = dragGhost.style.display; dragGhost.style.display = 'none'; }
        const el = document.elementFromPoint(x, y);
        if (dragGhost) dragGhost.style.display = prevDisplay;
        if (!el) return null;
        const slot = el.closest('.shortcut-slot');
        if (slot) return slot;
        const inList = el.closest('#anim-list');
        if (inList) return list;
        return null;
    }

    function handleDrop(card, target) {
        if (!target) return;
        const from = card.parentElement;
        if (target === list) {
            if (from && from.classList.contains('shortcut-slot')) {
                list.appendChild(card);
                refreshSlotStates();
                pushShortcuts();
            }
            return;
        }
        if (target.classList.contains('shortcut-slot')) {
            // Slot drop: swap or place
            const existing = target.querySelector('.anim-card');
            if (existing === card) return; // dropped on its own slot
            if (existing) {
                if (from && from.classList.contains('shortcut-slot')) {
                    // swap between slots
                    from.appendChild(existing);
                } else {
                    // dragged from list → existing returns to list
                    list.appendChild(existing);
                }
            }
            target.appendChild(card);
            refreshSlotStates();
            pushShortcuts();
        }
    }

    // Wire each card for pointer-based DnD. We add an additional click guard
    // that suppresses the click event right after a real drag.
    allCards.forEach(card => {
        // Prevent the native HTML5 DnD ghost when dragging is enabled in HTML.
        card.draggable = false;
        card.addEventListener('pointerdown', pointerDown);
        card.addEventListener('pointermove', pointerMove);
        card.addEventListener('pointerup', pointerUp);
        card.addEventListener('pointercancel', pointerCancel);
        card.addEventListener('click', (e) => {
            if (suppressNextClick) {
                suppressNextClick = false;
                e.preventDefault();
                e.stopImmediatePropagation();
            }
        }, true);
    });
})();
