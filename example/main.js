import { createCounter } from '@/counter.ts';
import confetti from 'canvas-confetti';
import React from 'react';
import ReactDOM from 'react-dom';
import RefreshCounter from './RefreshCounter.tsx';

const counter = createCounter(0);
const btn = document.getElementById('counter');

btn.addEventListener('click', () => {
  const count = counter.increment();
  btn.textContent = `Clicks: ${count}`;
});

ReactDOM.render(React.createElement(RefreshCounter), document.getElementById('react-counter'));

console.log('[example] App loaded:', import.meta.env.RUSH_APP_NAME || 'Rush Demo');
