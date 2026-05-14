// Sidebar navigation and page routing
const sidebar = document.getElementById('sidebar');
const sidebarToggle = document.getElementById('sidebar-toggle');
const sidebarOverlay = document.getElementById('sidebar-overlay');
const navItems = document.querySelectorAll('.nav-item');
const pages = document.querySelectorAll('.page');

// Map of page names to URL paths.
const PAGE_PATHS = {
    performance: '/',
    calibrate: '/calibrate',
    network: '/network',
    bluetooth: '/bluetooth',
};

// Legacy paths from the previous URL scheme — redirect to /.
const LEGACY_REDIRECTS = {
    '/animations': 'performance',
    '/audio': 'performance',
};

function closeSidebar() {
    sidebar.classList.remove('open');
    sidebarOverlay.classList.remove('open');
}
function openSidebar() {
    sidebar.classList.add('open');
    sidebarOverlay.classList.add('open');
}
sidebarToggle.addEventListener('click', () => {
    if (sidebar.classList.contains('open')) closeSidebar();
    else openSidebar();
});
sidebarOverlay.addEventListener('click', closeSidebar);

let currentPage = 'performance';

function showPage(pageName) {
    pages.forEach(p => p.classList.remove('active'));
    const el = document.getElementById('page-' + pageName);
    if (el) el.classList.add('active');
    navItems.forEach(item => {
        item.classList.toggle('active', item.dataset.page === pageName);
    });
    currentPage = pageName;
    document.body.dataset.page = pageName;

    // Page-load hooks
    if (pageName === 'bluetooth' && typeof ws !== 'undefined' && ws && ws.readyState === WebSocket.OPEN) {
        ws.send('bt:list');
    } else if (pageName === 'network' && typeof ws !== 'undefined' && ws && ws.readyState === WebSocket.OPEN) {
        ws.send('net:list');
    } else if (pageName === 'calibrate' && typeof ws !== 'undefined' && ws && ws.readyState === WebSocket.OPEN) {
        ws.send('get_calibration');
    } else if (pageName === 'performance' && typeof ws !== 'undefined' && ws && ws.readyState === WebSocket.OPEN) {
        ws.send('hw_audio:list');
    }
}

navItems.forEach(item => {
    item.addEventListener('click', () => {
        const page = item.dataset.page;
        if (!page) return;
        showPage(page);
        const path = PAGE_PATHS[page] || '/';
        if (window.location.pathname !== path) {
            history.pushState({ page }, '', path);
        }
        closeSidebar();
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
    if (LEGACY_REDIRECTS[path]) {
        history.replaceState({ page: LEGACY_REDIRECTS[path] }, '', PAGE_PATHS[LEGACY_REDIRECTS[path]]);
        showPage(LEGACY_REDIRECTS[path]);
        return;
    }
    const found = Object.entries(PAGE_PATHS).find(([, p]) => p === path);
    if (found) showPage(found[0]);
    else showPage('performance');
}
