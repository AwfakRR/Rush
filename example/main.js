import { createCounter } from './counter.ts';
import confetti from 'canvas-confetti';

const counter = createCounter(0);
const btn = document.getElementById('counter');

btn.addEventListener('click', () => {
  const count = counter.increment();
  btn.textContent = `Clicks: ${count}`;
});

console.log('[example] App loaded successfully.');
