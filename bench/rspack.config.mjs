import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { rspack } from '@rspack/core';
import { ReactRefreshRspackPlugin } from '@rspack/plugin-react-refresh';
import { app } from './generate.mjs';

export default (_, { mode = 'development' } = {}) => {
  const development = mode === 'development';
  const templateContent = readFileSync(join(app, 'index.html'), 'utf8')
    .replace('<script type="module" src="/src/main.tsx"></script>', '');
  return {
    mode,
    context: app,
    entry: './src/main.tsx',
    output: { path: join(app, 'dist') },
    devServer: { hot: true, static: false },
    module: {
      rules: [{
        test: /\.tsx$/,
        loader: 'builtin:swc-loader',
        options: {
          jsc: {
            parser: { syntax: 'typescript', tsx: true },
            transform: { react: { runtime: 'classic', development, refresh: development } },
          },
        },
      }],
    },
    plugins: [
      new rspack.HtmlRspackPlugin({ templateContent }),
      ...(development ? [new ReactRefreshRspackPlugin()] : []),
    ],
  };
};
