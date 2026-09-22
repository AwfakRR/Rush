let count = 0;
const btn = document.getElementById('counter');

btn.addEventListener('click', () => {
  count++;
  btn.textContent = `Clicks: ${count}`;
});

console.log('[example] App loaded successfully.');
