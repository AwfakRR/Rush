import React from 'react';
import { renderToString } from 'react-dom/server';
import { App } from './app.ts';

export function render() {
  return renderToString(React.createElement(App));
}
