import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './app.ts';
import './style.css';

ReactDOM.hydrateRoot(document.getElementById('app')!, React.createElement(App));
