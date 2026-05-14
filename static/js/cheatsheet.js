// Keyboard shortcuts cheatsheet overlay.

(function () {
    const overlay = document.getElementById('cheatsheet-overlay');
    const closeBtn = document.getElementById('cheatsheet-close');

    function setOpen(open) {
        if (!overlay) return;
        overlay.classList.toggle('open', open);
    }

    window.toggleCheatsheet = function (force) {
        if (!overlay) return;
        if (typeof force === 'boolean') { setOpen(force); return; }
        setOpen(!overlay.classList.contains('open'));
    };

    if (overlay) {
        overlay.addEventListener('click', (e) => {
            if (e.target === overlay) setOpen(false);
        });
    }
    if (closeBtn) closeBtn.addEventListener('click', () => setOpen(false));
})();
