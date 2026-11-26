import { execFile } from 'node:child_process';
import { mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { promisify } from 'node:util';

const exec = promisify(execFile);

export function createSsrLoader({ root = process.cwd(), origin, rush = 'rush' }) {
  root = resolve(root);
  const versionUrl = new URL('/__rush/ssr-version', origin);
  const entries = new Map();
  let directory;
  let closed = false;

  async function bundleDirectory() {
    const dir = await mkdtemp(join(tmpdir(), 'rush-ssr-'));
    try {
      await symlink(join(root, 'node_modules'), join(dir, 'node_modules'), 'junction');
      return dir;
    } catch (error) {
      await rm(dir, { recursive: true, force: true });
      throw error;
    }
  }

  return {
    async ssrLoadModule(entry) {
      if (closed) throw new Error('Rush SSR loader is closed');
      const file = resolve(root, entry);
      if (!entries.has(file)) {
        entries.set(file, { name: `${entries.size}.mjs`, pending: Promise.resolve() });
      }
      const record = entries.get(file);
      const pending = record.pending.catch(() => {}).then(async () => {
        const response = await fetch(versionUrl, { cache: 'no-store' });
        if (!response.ok) throw new Error(`Rush SSR version request failed: ${response.status}`);
        const version = await response.text();
        if (record.version === version) return record.module;

        const { stdout } = await exec(rush, ['ssr-transform', file, '--root', root], {
          maxBuffer: 16 * 1024 * 1024,
        });
        directory ??= bundleDirectory();
        const bundle = join(await directory, record.name);
        await writeFile(bundle, stdout);
        const url = pathToFileURL(bundle);
        url.searchParams.set('v', version);
        record.module = await import(url.href);
        record.version = version;
        return record.module;
      });
      record.pending = pending;
      return pending;
    },

    async close() {
      closed = true;
      await Promise.allSettled([...entries.values()].map(record => record.pending));
      if (directory) await rm(await directory, { recursive: true, force: true });
    },
  };
}
