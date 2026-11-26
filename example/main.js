import { createCounter } from '@/counter.ts';
import confetti from 'canvas-confetti';
import styles from './card.module.css';

document.getElementById('demo').classList.add(styles.card);
const counter = createCounter(0);
const btn = document.getElementById('counter');

btn.addEventListener('click', () => {
  const count = counter.increment();
  btn.textContent = `Clicks: ${count}`;
});

const appName = import.meta.hot && import.meta.env.RUSH_APP_NAME;
console.log('[example] App loaded:', appName || 'Rush Demo');
