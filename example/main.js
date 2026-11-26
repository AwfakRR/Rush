import { createCounter } from '@/counter.ts';
import confetti from 'canvas-confetti';
import styles from './card.module.css';
import welcome from 'virtual:welcome';
import pluginDemo from './plugin-demo.yaml';

document.getElementById('demo').classList.add(styles.card);
document.getElementById('plugin-demo').textContent = `${welcome} — ${pluginDemo.title} (${__DEMO_MODE__})`;
const counter = createCounter(0);
const btn = document.getElementById('counter');

btn.addEventListener('click', () => {
  const count = counter.increment();
  btn.textContent = `Clicks: ${count}`;
});

const lazyButton = document.getElementById('load-lazy');
const lazyStatus = document.getElementById('lazy-status');
lazyButton.addEventListener('click', async () => {
  lazyButton.disabled = true;
  lazyStatus.textContent = 'Loading lazy chunk…';
  try {
    const { filename } = await import('./lazy.js');
    lazyStatus.textContent = `Loaded on demand: ${filename}`;
  } catch {
    lazyStatus.textContent = 'Could not load the lazy chunk. Try again.';
    lazyButton.disabled = false;
  }
});

const appName = import.meta.hot && import.meta.env.RUSH_APP_NAME;
console.log('[example] App loaded:', appName || 'Rush Demo');
