// Sidebar navigation and page routing
const sidebar = document.getElementById('sidebar');
const sidebarToggle = document.getElementById('sidebar-toggle');
const sidebarOverlay = document.getElementById('sidebar-overlay');
const navItems = document.querySelectorAll('.nav-item');
const pages = document.querySelectorAll('.page');

// Map of page names to URL paths
const PAGE_PATHS = {
    animations: '/animations',
    audio: '/audio',
    calibrate: '/calibrate',
};

sidebarToggle.addEventListener('click', () => sidebar.classList.toggle('open'));
sidebarOverlay.addEventListener('click', () => sidebar.classList.remove('open'));

function showPage(pageName) {
    pages.forEach(p => p.classList.remove('active'));
    const el = document.getElementById('page-' + pageName);
    if (el) el.classList.add('active');
}

navItems.forEach(item => {
    item.addEventListener('click', () => {
        const page = item.dataset.page;
        const anim = item.dataset.anim;

        showPage(page);

        // Update URL
        const path = PAGE_PATHS[page] || '/';
        if (window.location.pathname !== path) {
            history.pushState({ page, anim }, '', path);
        }

        // Start animation
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send('animate:' + anim);
            setActiveAnim(anim);
        }

        sidebar.classList.remove('open');
    });
});

// Handle browser back/forward
window.addEventListener('popstate', (e) => {
    if (e.state && e.state.page) {
        showPage(e.state.page);
    } else {
        routeFromPath();
    }
});

// Route based on current URL path on page load
function routeFromPath() {
    const path = window.location.pathname;
    if (path === '/audio') {
        showPage('audio');
    } else if (path === '/calibrate') {
        showPage('calibrate');
        if (ws && ws.readyState === WebSocket.OPEN) ws.send('get_calibration');
    } else {
        showPage('animations');
    }
}
