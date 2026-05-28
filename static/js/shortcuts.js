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
    //
    // Mouse:  press and move past a small threshold starts the drag.
    // Touch:  a short long-press *arms* the drag — this leaves plain finger
    //         swipes free to scroll the (often long) animation list. Once
    //         armed, the pointer is captured and dragging near the top/bottom
    //         edge auto-scrolls the page so off-screen slots are reachable.

    const scrollContainer = document.querySelector('.content');

    let dragging = null;          // the source card element
    let dragGhost = null;         // floating clone following the pointer
    let dragOffsetX = 0;
    let dragOffsetY = 0;
    let dragStartX = 0;
    let dragStartY = 0;
    let dragStarted = false;       // true once the drag is actually armed
    let suppressNextClick = false;
    let dragPointerId = null;
    let dragPointerType = 'mouse';
    let lastClientX = 0;
    let lastClientY = 0;
    let longPressTimer = null;
    let autoScrollRAF = null;
    const DRAG_THRESHOLD = 6;      // mouse: px before a drag begins
    const TOUCH_SLOP = 10;         // touch: movement that counts as a scroll, not a hold
    const LONG_PRESS_MS = 240;     // touch: hold duration that arms a drag
    const EDGE = 64;               // auto-scroll trigger zone near top/bottom
    const EDGE_SPEED = 16;         // max auto-scroll px per frame

    function clearLongPress() {
        if (longPressTimer !== null) { clearTimeout(longPressTimer); longPressTimer = null; }
    }

    function buildGhost() {
        const r = dragging.getBoundingClientRect();
        dragGhost = dragging.cloneNode(true);
        dragGhost.classList.add('drag-ghost');
        dragGhost.style.position = 'fixed';
        dragGhost.style.left = r.left + 'px';
        dragGhost.style.top = r.top + 'px';
        dragGhost.style.width = r.width + 'px';
        dragGhost.style.margin = '0';
        dragGhost.style.pointerEvents = 'none';
        dragGhost.style.zIndex = '1000';
        dragGhost.style.opacity = '0.9';
        dragGhost.style.transform = 'scale(0.96)';
        dragGhost.style.boxShadow = '0 12px 32px rgba(0,0,0,0.6), 0 0 18px var(--neon-magenta-glow)';
        document.body.appendChild(dragGhost);
        dragging.classList.add('dragging');
    }

    function moveGhost() {
        if (!dragGhost) return;
        dragGhost.style.left = (lastClientX - dragOffsetX) + 'px';
        dragGhost.style.top = (lastClientY - dragOffsetY) + 'px';
    }

    function highlightTarget() {
        const target = dropTargetFromPoint(lastClientX, lastClientY);
        document.querySelectorAll('.drop-target').forEach(el => el.classList.remove('drop-target'));
        if (target) target.classList.add('drop-target');
    }

    // Arm the drag: capture the pointer and show the floating ghost.
    function armDrag() {
        if (!dragging || dragStarted) return;
        clearLongPress();
        dragStarted = true;
        try { dragging.setPointerCapture(dragPointerId); } catch (_) {}
        buildGhost();
        moveGhost();
        highlightTarget();
        if (dragPointerType === 'touch' && navigator.vibrate) {
            try { navigator.vibrate(12); } catch (_) {}
        }
    }

    // Auto-scroll the page while a drag hovers near the top/bottom edge.
    function autoScrollTick() {
        autoScrollRAF = null;
        if (!dragStarted || !scrollContainer) return;
        const rect = scrollContainer.getBoundingClientRect();
        let dv = 0;
        if (lastClientY < rect.top + EDGE) {
            dv = -EDGE_SPEED * ((rect.top + EDGE - lastClientY) / EDGE);
        } else if (lastClientY > rect.bottom - EDGE) {
            dv = EDGE_SPEED * ((lastClientY - (rect.bottom - EDGE)) / EDGE);
        }
        if (dv !== 0) {
            scrollContainer.scrollTop += dv;
            highlightTarget();
            autoScrollRAF = requestAnimationFrame(autoScrollTick);
        }
    }
    function maybeAutoScroll() {
        if (autoScrollRAF === null) autoScrollRAF = requestAnimationFrame(autoScrollTick);
    }
    function stopAutoScroll() {
        if (autoScrollRAF !== null) { cancelAnimationFrame(autoScrollRAF); autoScrollRAF = null; }
    }

    // Fully abandon the current gesture without performing a drop (e.g. a touch
    // that turned into a scroll).
    function resetDrag() {
        clearLongPress();
        stopAutoScroll();
        document.querySelectorAll('.drop-target').forEach(el => el.classList.remove('drop-target'));
        if (dragGhost && dragGhost.parentElement) dragGhost.parentElement.removeChild(dragGhost);
        dragGhost = null;
        if (dragging) dragging.classList.remove('dragging');
        dragging = null;
        dragStarted = false;
        dragPointerId = null;
    }

    function pointerDown(e) {
        if (e.button !== undefined && e.button !== 0) return;
        const card = e.currentTarget;
        dragging = card;
        dragStarted = false;
        dragPointerId = e.pointerId;
        dragPointerType = e.pointerType || 'mouse';
        const r = card.getBoundingClientRect();
        dragOffsetX = e.clientX - r.left;
        dragOffsetY = e.clientY - r.top;
        dragStartX = lastClientX = e.clientX;
        dragStartY = lastClientY = e.clientY;
        if (dragPointerType === 'touch' || dragPointerType === 'pen') {
            // Don't capture yet — let the list scroll. A held finger arms the drag.
            clearLongPress();
            longPressTimer = setTimeout(armDrag, LONG_PRESS_MS);
        } else {
            // Mouse: capture immediately; the move threshold decides if it's a drag.
            try { card.setPointerCapture(e.pointerId); } catch (_) {}
        }
    }

    function pointerMove(e) {
        if (!dragging) return;
        lastClientX = e.clientX;
        lastClientY = e.clientY;
        const dx = e.clientX - dragStartX;
        const dy = e.clientY - dragStartY;

        if (!dragStarted) {
            if (dragPointerType === 'touch' || dragPointerType === 'pen') {
                // Movement before the hold fires = the user is scrolling. Bail.
                if (Math.hypot(dx, dy) > TOUCH_SLOP) resetDrag();
                return;
            }
            // Mouse: begin the drag once past the threshold.
            if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
            armDrag();
        }

        e.preventDefault();
        moveGhost();
        highlightTarget();
        maybeAutoScroll();
    }

    function pointerUp(e) {
        clearLongPress();
        stopAutoScroll();
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
        dragPointerId = null;
    }

    function pointerCancel() {
        resetDrag();
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
