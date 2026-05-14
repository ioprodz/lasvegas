// WebSocket connection management
let ws;

function connect() {
    const host = window.location.hostname;
    ws = new WebSocket(`ws://${host}:8080`);
    ws.binaryType = 'arraybuffer';
    ws.onopen = () => {
        if (typeof setConnectionStatus === 'function') setConnectionStatus(true);
        ws.send('hw_audio:list');
        // Re-fire the current page's initial data request, in case the
        // route resolved before the socket was open.
        const page = document.body.dataset.page;
        if (page === 'bluetooth')      ws.send('bt:list');
        else if (page === 'network')   ws.send('net:list');
        else if (page === 'calibrate') ws.send('get_calibration');
    };
    ws.onclose = () => {
        if (typeof setConnectionStatus === 'function') setConnectionStatus(false);
        if (typeof setActiveAnim === 'function') setActiveAnim(null);
        setTimeout(connect, 2000);
    };
    ws.onmessage = handleWsMessage;
}
