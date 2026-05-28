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

    // ── Assigning animations to slots ────────────────────────────────────
    //
    // Desktop (mouse): drag a card onto a slot — the familiar DnD.
    // Touch (phones/tablets): tap a card to play it; long-press to open a
    //   bottom sheet that assigns it to a slot via a mini 3×3 grid. Dragging
    //   on touch is too fiddly, so it's disabled there — plain swipes scroll
    //   the (long) animation list as normal.

    // ── Bottom sheet (touch assign) ──────────────────────────────────────
    const sheetOverlay = document.getElementById('slot-sheet-overlay');
    const sheetGrid    = document.getElementById('slot-sheet-grid');
    const sheetTitle   = document.getElementById('slot-sheet-title');
    const sheetPlay    = document.getElementById('slot-sheet-play');
    const sheetRemove  = document.getElementById('slot-sheet-remove');
    const sheetCancel  = document.getElementById('slot-sheet-cancel');
    let sheetCard = null;
    let sheetCloseTimer = null;

    function cardLabel(card) {
        const h = card.querySelector('h3');
        return h ? h.textContent.trim() : card.dataset.anim;
    }
    function cardSlotIndex(card) {
        const p = card.parentElement;
        return p && p.classList.contains('shortcut-slot') ? parseInt(p.dataset.slot, 10) : -1;
    }

    function openSlotSheet(card) {
        if (!sheetOverlay) return;
        sheetCard = card;
        sheetTitle.textContent = cardLabel(card);

        // Rebuild the mini grid from the live slot contents each time.
        sheetGrid.innerHTML = '';
        slotEls.forEach((slot, idx) => {
            const occupant = slot.querySelector('.anim-card');
            const cell = document.createElement('button');
            cell.type = 'button';
            cell.className = 'slot-sheet-cell';
            if (occupant === card) cell.classList.add('current');
            else if (!occupant) cell.classList.add('empty');

            const num = document.createElement('span');
            num.className = 'slot-sheet-cell-num';
            num.textContent = String(idx + 1);
            const name = document.createElement('span');
            name.className = 'slot-sheet-cell-name';
            name.textContent = occupant ? cardLabel(occupant) : '—';
            cell.append(num, name);

            cell.addEventListener('click', () => {
                if (occupant !== card) handleDrop(card, slot);
                closeSlotSheet();
            });
            sheetGrid.appendChild(cell);
        });

        // Offer removal only when this card is already in a slot.
        const cur = cardSlotIndex(card);
        if (cur >= 0) {
            sheetRemove.hidden = false;
            sheetRemove.textContent = 'Remove from slot ' + (cur + 1);
        } else {
            sheetRemove.hidden = true;
        }

        if (sheetCloseTimer) { clearTimeout(sheetCloseTimer); sheetCloseTimer = null; }
        sheetOverlay.hidden = false;
        requestAnimationFrame(() => sheetOverlay.classList.add('open'));
    }

    function closeSlotSheet() {
        if (!sheetOverlay) return;
        sheetOverlay.classList.remove('open');
        sheetCard = null;
        sheetCloseTimer = setTimeout(() => { sheetOverlay.hidden = true; }, 220);
    }

    if (sheetOverlay) {
        sheetPlay.addEventListener('click', () => {
            const c = sheetCard;
            closeSlotSheet();
            if (c) c.click();            // reuse the normal play handler
        });
        sheetRemove.addEventListener('click', () => {
            const c = sheetCard;
            if (c) handleDrop(c, list);  // move back to the library
            closeSlotSheet();
        });
        sheetCancel.addEventListener('click', closeSlotSheet);
        // Close on a backdrop tap that both starts and ends on the backdrop,
        // so the click trailing the opening long-press can't dismiss it.
        let downOnBackdrop = false;
        sheetOverlay.addEventListener('pointerdown', (e) => { downOnBackdrop = (e.target === sheetOverlay); });
        sheetOverlay.addEventListener('click', (e) => {
            if (e.target === sheetOverlay && downOnBackdrop) closeSlotSheet();
        });
    }

    // Swallow the single click that fires right after the opening long-press
    // so it doesn't fall through to the sheet or replay the animation.
    function swallowNextClick() {
        const cleanup = () => {
            document.removeEventListener('click', handler, true);
            clearTimeout(safety);
        };
        const handler = (e) => { e.stopPropagation(); e.preventDefault(); cleanup(); };
        const safety = setTimeout(cleanup, 500);
        document.addEventListener('click', handler, true);
    }

    // ── Pointer drag (mouse) + long-press (touch) ────────────────────────
    let dragging = null;          // the source card element
    let dragGhost = null;         // floating clone following the pointer (mouse)
    let dragOffsetX = 0;
    let dragOffsetY = 0;
    let dragStartX = 0;
    let dragStartY = 0;
    let dragStarted = false;
    let suppressNextClick = false;
    let dragPointerType = 'mouse';
    let longPressTimer = null;
    let longPressFired = false;
    const DRAG_THRESHOLD = 6;      // mouse: px before a drag begins
    const TOUCH_SLOP = 10;         // touch: movement that counts as a scroll, not a hold
    const LONG_PRESS_MS = 300;     // touch: hold duration that opens the sheet

    const isTouch = (t) => t === 'touch' || t === 'pen';

    function clearLongPress() {
        if (longPressTimer !== null) { clearTimeout(longPressTimer); longPressTimer = null; }
    }

    // Touch hold completed: mark it armed (sheet opens on release) + nudge.
    function onLongPress() {
        longPressTimer = null;
        longPressFired = true;
        if (dragging) dragging.classList.add('longpress-armed');
        if (navigator.vibrate) { try { navigator.vibrate(12); } catch (_) {} }
    }

    function pointerDown(e) {
        if (e.button !== undefined && e.button !== 0) return;
        const card = e.currentTarget;
        dragging = card;
        dragStarted = false;
        longPressFired = false;
        dragPointerType = e.pointerType || 'mouse';
        const r = card.getBoundingClientRect();
        dragOffsetX = e.clientX - r.left;
        dragOffsetY = e.clientY - r.top;
        dragStartX = e.clientX;
        dragStartY = e.clientY;
        if (isTouch(dragPointerType)) {
            // Don't capture — leave the list free to scroll. A held finger
            // opens the assign sheet on release.
            clearLongPress();
            longPressTimer = setTimeout(onLongPress, LONG_PRESS_MS);
        } else {
            try { card.setPointerCapture(e.pointerId); } catch (_) {}
        }
    }

    function pointerMove(e) {
        if (!dragging) return;
        const dx = e.clientX - dragStartX;
        const dy = e.clientY - dragStartY;

        if (isTouch(dragPointerType)) {
            // Movement before the hold completes = a scroll — let it go.
            if (!longPressFired && Math.hypot(dx, dy) > TOUCH_SLOP) {
                clearLongPress();
                dragging.classList.remove('longpress-armed');
                dragging = null;
            }
            return;   // never drag on touch
        }

        // Mouse drag.
        if (!dragStarted) {
            if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
            dragStarted = true;
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
        dragGhost.style.left = (e.clientX - dragOffsetX) + 'px';
        dragGhost.style.top = (e.clientY - dragOffsetY) + 'px';
        const target = dropTargetFromPoint(e.clientX, e.clientY);
        document.querySelectorAll('.drop-target').forEach(el => el.classList.remove('drop-target'));
        if (target) target.classList.add('drop-target');
    }

    function pointerUp(e) {
        clearLongPress();
        if (!dragging) return;
        const card = dragging;

        if (isTouch(dragPointerType)) {
            card.classList.remove('longpress-armed');
            if (longPressFired) {
                openSlotSheet(card);
                swallowNextClick();
            }
            dragging = null;
            longPressFired = false;
            return;
        }

        // Mouse drop.
        const wasDragging = dragStarted;
        try { card.releasePointerCapture(e.pointerId); } catch (_) {}
        if (wasDragging) {
            const target = dropTargetFromPoint(e.clientX, e.clientY);
            handleDrop(card, target);
            suppressNextClick = true;
            setTimeout(() => { suppressNextClick = false; }, 50);
        }
        document.querySelectorAll('.drop-target').forEach(el => el.classList.remove('drop-target'));
        if (dragGhost && dragGhost.parentElement) dragGhost.parentElement.removeChild(dragGhost);
        dragGhost = null;
        card.classList.remove('dragging');
        dragging = null;
        dragStarted = false;
    }

    function pointerCancel() {
        clearLongPress();
        document.querySelectorAll('.drop-target').forEach(el => el.classList.remove('drop-target'));
        if (dragGhost && dragGhost.parentElement) dragGhost.parentElement.removeChild(dragGhost);
        dragGhost = null;
        if (dragging) dragging.classList.remove('dragging', 'longpress-armed');
        dragging = null;
        dragStarted = false;
        longPressFired = false;
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
        // Suppress the OS context menu / text-callout on a long press.
        card.addEventListener('contextmenu', (e) => e.preventDefault());
        card.addEventListener('click', (e) => {
            if (suppressNextClick) {
                suppressNextClick = false;
                e.preventDefault();
                e.stopImmediatePropagation();
            }
        }, true);
    });
})();
