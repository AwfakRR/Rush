import { mkdir, rm, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

export const bench = fileURLToPath(new URL('.', import.meta.url));
export const app = join(bench, 'app');
export const defaultComponents = '1000';

export function integer(value, flag, min) {
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(Number(value)) || Number(value) < min) {
    throw new Error(`${flag} must be an integer greater than or equal to ${min}`);
  }
  return Number(value);
}

export async function generate(count) {
  const source = join(app, 'src');
  const modules = join(app, 'node_modules');
  await rm(app, { recursive: true, force: true });
  await mkdir(source, { recursive: true });
  await mkdir(join(modules, '.bin'), { recursive: true });
  for (const name of ['react', 'react-dom', '.bin/esbuild']) {
    await symlink(join(bench, 'node_modules', name), join(modules, name));
  }

  const name = (i) => i === 0 ? 'App' : `Component${i}`;
  const componentFile = (i) => join(source, `${name(i)}.tsx`);
  const markers = {
    root: { file: componentFile(0), id: 'root-marker', original: 'root:initial', updated: 'root:edited' },
    leaf: { file: componentFile(count - 1), id: 'leaf-marker', original: 'leaf:initial', updated: 'leaf:edited' },
  };

  await writeFile(join(app, 'index.html'), `<!doctype html>
<html lang="en">
  <head><meta charset="utf-8"><title>Dev server benchmark</title></head>
  <body><div id="app"></div><script type="module" src="/src/main.tsx"></script></body>
</html>
`);
  await writeFile(join(source, 'main.tsx'), `/** @jsxRuntime classic */
import React from 'react';
import ReactDOM from 'react-dom';
import App from './App.tsx';

ReactDOM.render(<App />, document.getElementById('app'));
`);
  for (let i = 0; i < count; i++) {
    const children = [2 * i + 1, 2 * i + 2].filter((child) => child < count);
    const imports = children.map((child) => `import ${name(child)} from './${name(child)}.tsx';\n`).join('');
    const elements = children.map((child) => `<${name(child)} />`).join('');
    let body;
    if (i === 0) {
      body = `  const [counter, setCounter] = React.useState<number>(0);
  return <main><h1 id="${markers.root.id}">${markers.root.original}</h1>
    <button id="counter" onClick={() => setCounter(counter + 1)}>{counter}</button>${elements}</main>;`;
    } else if (i === count - 1) {
      body = `  return <span data-leaf id="${markers.leaf.id}">${markers.leaf.original}</span>;`;
    } else if (children.length === 0) {
      body = `  const index: number = ${i};
  return <span data-leaf>{index}</span>;`;
    } else {
      body = `  return <section>${elements}</section>;`;
    }
    await writeFile(componentFile(i), `/** @jsxRuntime classic */
import React from 'react';
${imports}
export default function ${name(i)}() {
${body}
}
`);
  }
  return { leaves: Math.ceil(count / 2), markers };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({
    options: { components: { type: 'string', default: defaultComponents } },
  });
  const count = integer(values.components, '--components', 2);
  await generate(count);
  console.log(`Generated ${count} components in ${app}`);
}
