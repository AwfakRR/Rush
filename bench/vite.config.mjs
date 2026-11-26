import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';
import { app } from './generate.mjs';

export default defineConfig({
  root: app,
  plugins: [react({ jsxRuntime: 'classic' })],
  cacheDir: 'node_modules/.vite',
});
