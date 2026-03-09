// LED grid preview
const gridContainer = document.getElementById('grid');
const gridLeds = [];
for (let i = 0; i < 360; i++) {
    const led = document.createElement('div');
    led.className = 'led';
    gridContainer.appendChild(led);
    gridLeds.push(led);
}

const previewToggle = document.getElementById('preview-toggle');
const mainEl = document.getElementById('main');

previewToggle.addEventListener('click', () => {
    mainEl.classList.toggle('preview-top');
    mainEl.classList.toggle('preview-right');
});

function updateGrid(data) {
    for (let i = 0; i < 360 && i * 3 + 2 < data.length; i++) {
        gridLeds[i].style.background = `rgb(${data[i*3]},${data[i*3+1]},${data[i*3+2]})`;
    }
}
