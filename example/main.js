import { createCounter } from '@/counter.ts';
import confetti from 'canvas-confetti';
import React from 'react';
import ReactDOM from 'react-dom';
import RefreshCounter from './RefreshCounter.tsx';
import styles from './card.module.css';

document.getElementById('demo').classList.add(styles.card);
const counter = createCounter(0);
const btn = document.getElementById('counter');

btn.addEventListener('click', () => {
  const count = counter.increment();
  btn.textContent = `Clicks: ${count}`;
});

ReactDOM.render(React.createElement(RefreshCounter), document.getElementById('react-counter'));

const appName = import.meta.hot && import.meta.env.RUSH_APP_NAME;
console.log('[example] App loaded:', appName || 'Rush Demo');
