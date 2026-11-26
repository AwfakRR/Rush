import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { availableParallelism, cpus, release, totalmem } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { parseArgs } from 'node:util';
import { chromium } from 'playwright';
import { app, bench, defaultComponents, generate, integer } from './generate.mjs';

const repo = join(bench, '..');
const timeout = 60_000;
const host = '127.0.0.1';
const rush = join(repo, 'target', 'release', 'rush');
const bin = (name) => join(bench, 'node_modules', '.bin', name);
const tools = [
  {
    name: 'Rush',
    command: (port) => [rush, '--root', app, '--port', String(port)],
    caches: ['node_modules/.rush'],
  },
  {
    name: 'Vite',
    command: (port) => [bin('vite'), '--config', 'vite.config.mjs', '--host', host, '--port', String(port), '--strictPort'],
    caches: ['node_modules/.vite'],
  },
  {
    name: 'Rspack',
    command: (port) => [bin('rspack'), 'serve', '--config', 'rspack.config.mjs', '--host', host, '--port', String(port)],
    caches: ['node_modules/.cache/rspack'],
  },
];

async function freePort() {
  const server = createServer().listen(0, host);
  await once(server, 'listening');
  const { port } = server.address();
  await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  return port;
}

async function waitForHttp(url, server, startedAt) {
  while (!server.exited && performance.now() - startedAt < timeout) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1000) });
      const respondedAt = performance.now();
      if (response.ok && (await response.text()).includes('<title>Dev server benchmark</title>')) {
        return respondedAt - startedAt;
      }
      await response.body?.cancel().catch(() => {});
    } catch {
      // The listener may not be bound yet, or the initial compilation may still be running.
    }
    await delay(10);
  }
  throw new Error(server.exited ? 'Server exited before serving the fixture' : 'Timed out waiting for fixture HTML');
}

async function waitForRender(page, expected, leaves) {
  const handle = await page.waitForFunction(({ expected, leaves }) =>
    Object.entries(expected).every(([id, text]) => document.getElementById(id)?.textContent === text) &&
    document.querySelectorAll('[data-leaf]').length === leaves,
  { expected, leaves });
  await handle.dispose();
}

async function measureEdit(page, marker, source, expected, leaves) {
  const counter = page.locator('#counter');
  await page.waitForFunction(() => window.__benchmarkSocketOpen);
  await counter.click();
  const stateBefore = await counter.textContent();
  let reloads = 0;
  const navigated = (frame) => { if (frame === page.mainFrame()) reloads++; };
  page.on('framenavigated', navigated);
  try {
    const before = performance.now();
    await writeFile(marker.file, source.replace(marker.original, marker.updated));
    expected[marker.id] = marker.updated;
    await waitForRender(page, expected, leaves);
    const ms = performance.now() - before;
    return { ms, reloads, statePreserved: await counter.textContent() === stateBefore };
  } finally {
    page.off('framenavigated', navigated);
  }
}

async function checkSyntaxRecovery(context, page, url, marker, source, expected, leaves) {
  const overlay = (p) => p.locator('#__rush_error');
  const counter = (p) => p.locator('#counter');
  const count = await counter(page).textContent();
  let reloads = 0;
  page.on('framenavigated', (frame) => { if (frame === page.mainFrame()) reloads++; });

  const working = source.replace(marker.original, marker.updated);
  const broken = working.replace('  const [counter', '  const broken = ;\n  const [counter');
  assert.notEqual(broken, working);
  await writeFile(marker.file, broken);
  await overlay(page).waitFor({ state: 'visible' });
  assert.match(await overlay(page).textContent(), /Unexpected token/);
  assert.equal(await counter(page).textContent(), count);
  assert.equal(reloads, 0);

  const cold = await context.newPage();
  await cold.goto(url);
  await overlay(cold).waitFor({ state: 'visible' });
  assert.equal(await counter(cold).count(), 0);
  let coldReloads = 0;
  cold.on('framenavigated', (frame) => { if (frame === cold.mainFrame()) coldReloads++; });

  const fixedMarker = `${marker.updated}:fixed`;
  await writeFile(marker.file, working.replace(marker.updated, fixedMarker));
  const recovered = { ...expected, [marker.id]: fixedMarker };
  await waitForRender(page, recovered, leaves);
  await overlay(page).waitFor({ state: 'detached' });
  assert.equal(await counter(page).textContent(), count);
  assert.equal(reloads, 0);

  await waitForRender(cold, recovered, leaves);
  assert.equal(await counter(cold).textContent(), '0');
  assert.equal(coldReloads, 1);
  await overlay(cold).waitFor({ state: 'detached' });
  console.log('Rush syntax recovery: live page kept counter and cleared overlay without reload; initially broken page mounted after one reload.');
}

async function measure(browser, tool, round, fixture, sources, checkRecovery) {
  const { markers, leaves } = fixture;
  for (const dir of tool.caches) {
    await rm(join(app, dir), { recursive: true, force: true });
  }
  for (const key of Object.keys(markers)) {
    await writeFile(markers[key].file, sources[key]);
  }
  const port = await freePort();
  const url = `http://${host}:${port}/`;
  const [command, ...args] = tool.command(port);
  const context = await browser.newContext();
  try {
    await context.addInitScript(() => {
      window.WebSocket = class extends WebSocket {
        constructor(...args) {
          super(...args);
          this.addEventListener('open', () => { window.__benchmarkSocketOpen = true; });
        }
      };
    });
    const page = await context.newPage();
    page.setDefaultTimeout(timeout);
    const browserErrors = [];
    page.on('pageerror', (error) => browserErrors.push(error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') browserErrors.push(message.text());
    });
    const expected = Object.fromEntries(Object.values(markers).map(({ id, original }) => [id, original]));

    const startedAt = performance.now();
    const child = spawn(command, args, { cwd: bench, stdio: ['ignore', 'pipe', 'pipe'] });
    let logs = '';
    const append = (data) => { logs = (logs + data).slice(-6000); };
    child.stdout.on('data', append);
    child.stderr.on('data', append);
    const server = { exited: false };
    const exited = new Promise((resolve) => {
      child.once('exit', () => { server.exited = true; resolve(); });
      child.once('error', (error) => { append(error.message); server.exited = true; resolve(); });
    });
    try {
      const startupMs = await waitForHttp(url, server, startedAt);
      await page.goto(url, { waitUntil: 'commit' });
      await waitForRender(page, expected, leaves);
      const coldMs = performance.now() - startedAt;
      const root = await measureEdit(page, markers.root, sources.root, expected, leaves);
      const leaf = await measureEdit(page, markers.leaf, sources.leaf, expected, leaves);
      if (checkRecovery) {
        await checkSyntaxRecovery(context, page, url, markers.root, sources.root, expected, leaves);
      }
      return { round, startupMs, coldMs, root, leaf };
    } catch (cause) {
      throw new Error(`${tool.name}, run ${round}: ${cause.message}\nServer output:\n${logs}\nBrowser errors:\n${browserErrors.slice(-12).join('\n')}`, { cause });
    } finally {
      if (!server.exited) child.kill();
      await Promise.race([exited, delay(2000, undefined, { ref: false })]);
      if (!server.exited) {
        child.kill('SIGKILL');
        await exited;
      }
    }
  } finally {
    await context.close();
  }
}

function stats(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return {
    median: sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2,
    min: sorted[0],
    max: sorted.at(-1),
  };
}

function summarize(samples) {
  return {
    startupMs: stats(samples.map(({ startupMs }) => startupMs)),
    coldMs: stats(samples.map(({ coldMs }) => coldMs)),
    ...Object.fromEntries(['root', 'leaf'].map((key) => [key, {
      ms: stats(samples.map((sample) => sample[key].ms)),
      reloadRuns: samples.filter((sample) => sample[key].reloads > 0).length,
      statePreservedRuns: samples.filter((sample) => sample[key].statePreserved).length,
    }])),
  };
}

function table(summary, runs) {
  const fmt = ({ median, min, max }) => `${median.toFixed(1)} (${min.toFixed(1)}–${max.toFixed(1)})`;
  const hmr = ({ ms, reloadRuns, statePreservedRuns }) =>
    `${fmt(ms)}; ${reloadRuns}/${runs} reloads; ${statePreservedRuns}/${runs} state kept`;
  return [
    '| Tool | HTTP ready (ms) | Cold page (ms) | Root edit (ms) | Leaf edit (ms) |',
    '| --- | ---: | ---: | ---: | ---: |',
    ...Object.entries(summary).map(([name, { startupMs, coldMs, root, leaf }]) =>
      `| ${name} | ${fmt(startupMs)} | ${fmt(coldMs)} | ${hmr(root)} | ${hmr(leaf)} |`),
  ].join('\n');
}

async function metadata(browser) {
  const { devDependencies } = JSON.parse(await readFile(join(bench, 'package.json'), 'utf8'));
  const packages = {};
  for (const name of Object.keys(devDependencies)) {
    const installed = JSON.parse(await readFile(join(bench, 'node_modules', name, 'package.json'), 'utf8'));
    packages[name] = installed.version;
  }
  const cargo = await readFile(join(repo, 'Cargo.toml'), 'utf8');
  const command = (name, args) => execFileSync(name, args, { cwd: repo, encoding: 'utf8' }).trim();
  return {
    machine: {
      cpu: cpus()[0]?.model,
      logicalCores: cpus().length,
      availableCores: availableParallelism(),
      memoryBytes: totalmem(),
      os: process.platform,
      release: release(),
      arch: process.arch,
    },
    versions: {
      node: process.version,
      rust: command('rustc', ['--version']),
      chromium: browser.version(),
      rush: cargo.match(/^version = "([^"]+)"/m)[1],
      rushCommit: command('git', ['rev-parse', 'HEAD']),
      packages,
    },
  };
}

async function main() {
  const { values } = parseArgs({
    options: {
      components: { type: 'string', default: defaultComponents },
      runs: { type: 'string', default: '5' },
      'assert-rush-refresh': { type: 'boolean', default: false },
    },
  });
  const components = integer(values.components, '--components', 2);
  const runs = integer(values.runs, '--runs', 1);
  const started = new Date().toISOString();
  console.log(`Benchmarking ${components} components, ${runs} runs per tool`);
  execFileSync('cargo', ['build', '--locked', '--release'], { cwd: repo, stdio: 'inherit' });
  const fixture = await generate(components);
  const sources = Object.fromEntries(await Promise.all(
    Object.entries(fixture.markers).map(async ([key, { file }]) => [key, await readFile(file, 'utf8')]),
  ));
  const samples = Object.fromEntries(tools.map(({ name }) => [name, []]));
  const browser = await chromium.launch();
  let environment;
  try {
    environment = await metadata(browser);
    for (let i = 0; i < runs; i++) {
      for (let j = 0; j < tools.length; j++) {
        const tool = tools[(i + j) % tools.length];
        console.log(`Run ${i + 1}/${runs}: ${tool.name}`);
        const checkRecovery = values['assert-rush-refresh'] && i === 0 && tool.name === 'Rush';
        samples[tool.name].push(await measure(browser, tool, i + 1, fixture, sources, checkRecovery));
      }
    }
  } finally {
    await browser.close();
  }
  const summary = Object.fromEntries(Object.entries(samples).map(([name, data]) => [name, summarize(data)]));
  const output = join(bench, 'results', `${started.replace(/[:.]/g, '-')}.json`);
  await mkdir(join(bench, 'results'), { recursive: true });
  await writeFile(output, JSON.stringify({ started, components, leaves: fixture.leaves, runs, ...environment, samples, summary }, null, 2) + '\n');
  console.log(`\n${table(summary, runs)}\n\nResults: ${output}`);
  if (values['assert-rush-refresh']) {
    for (const [edit, result] of Object.entries({ root: summary.Rush.root, leaf: summary.Rush.leaf })) {
      if (result.reloadRuns !== 0 || result.statePreservedRuns !== runs) {
        throw new Error(`Rush ${edit} edit did not preserve state without reloads in every run`);
      }
    }
    console.log('Rush React Fast Refresh passed for every root and leaf edit.');
  }
}

await main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
