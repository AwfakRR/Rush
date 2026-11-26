const { createRequire: pluginRequire } = require('node:module');
const { Console } = require('node:console');
const pluginFs = require('node:fs/promises');
const pluginPath = require('node:path');
const { pathToFileURL } = require('node:url');
global.console = new Console(process.stderr);

function hookOrder(order) {
  return order === 'pre' ? -1 : order === 'post' ? 1 : 0;
}

function mergePluginConfig(base, extra) {
  for (const [key, value] of Object.entries(extra ?? {})) {
    if (value === undefined) continue;
    if (Array.isArray(value) && Array.isArray(base[key])) base[key] = [...base[key], ...value];
    else if (value && Object.getPrototypeOf(value) === Object.prototype) {
      const previous = base[key];
      base[key] = mergePluginConfig(previous && Object.getPrototypeOf(previous) === Object.prototype
        && !Array.isArray(previous) ? { ...previous } : {}, value);
    } else base[key] = value;
  }
  return base;
}

async function loadRushPlugins(root, configFile, command, ssr = false) {
  const development = command === 'serve';
  const mode = development ? 'development' : 'production';
  const env = { command, mode, isSsrBuild: ssr && !development, isPreview: false };
  const requireFromRoot = pluginRequire(pluginPath.join(root, 'package.json'));
  const imported = (await import(pathToFileURL(configFile).href)).default;
  const userConfig = await (typeof imported === 'function' ? imported(env) : imported);
  if (!userConfig || typeof userConfig !== 'object' || Array.isArray(userConfig)) {
    throw new Error('plugin config must default-export a config object or function');
  }

  async function flatten(input) {
    const value = await input;
    return Array.isArray(value) ? (await Promise.all(value.map(flatten))).flat() : value ? [value] : [];
  }
  let config = { root, base: '/', ...userConfig };
  const sorted = (await flatten(config.plugins)).sort((a, b) => hookOrder(a.enforce) - hookOrder(b.enforce));
  const plugins = [];
  for (const plugin of sorted) {
    if (typeof plugin.apply === 'function' ? await plugin.apply(config, env) :
      !plugin.apply || plugin.apply === command) plugins.push(plugin);
  }

  const middlewares = [];
  const assets = [];
  let fallbackResolve = async (source, importer) => {
    const from = importer && pluginPath.isAbsolute(importer) ? importer : pluginPath.join(root, 'index.html');
    if (source.startsWith('.') || pluginPath.isAbsolute(source)) {
      const [name, suffix] = splitPluginId(source);
      const target = pluginPath.isAbsolute(name) ? name : pluginPath.resolve(pluginPath.dirname(from), name);
      const candidates = pluginPath.isAbsolute(name) ? [target, pluginPath.join(root, name)] : [target];
      for (const candidate of candidates) {
        for (const file of [candidate, ...['.js', '.ts', '.jsx', '.tsx', '.json'].map(ext => candidate + ext)]) {
          if ((await pluginFs.stat(file).catch(() => null))?.isFile()) return { id: file + suffix };
        }
      }
      return null;
    }
    try {
      return { id: requireFromRoot.resolve(source, { paths: [pluginPath.dirname(from)] }) };
    } catch (error) {
      if (error.code !== 'MODULE_NOT_FOUND') throw error;
      return null;
    }
  };
  const hooks = (name) => plugins
    .filter(plugin => typeof (plugin[name]?.handler ?? plugin[name]) === 'function')
    .sort((a, b) => hookOrder(a[name].order) - hookOrder(b[name].order));
  const message = value => value?.message ?? String(value);
  const filename = asset => asset.fileName ?? `assets/${asset.ref}-${pluginPath.basename(asset.name ?? 'asset')}`;
  const assetFor = ref => {
    const asset = assets.find(asset => asset.ref === ref);
    if (!asset) throw new Error(`unknown emitted file reference ${ref}`);
    return asset;
  };

  async function invoke(plugin, name, args, callData) {
    const context = {
      meta: { watchMode: development },
      async resolve(source, importer, options = {}) {
        const current = options.skipSelf === false ? undefined : plugin;
        const result = await resolve(source, importer, options, current, callData);
        return result ?? fallbackResolve(source, importer, options, callData);
      },
      warn(value) { console.error(`[${plugin.name}] ${message(value)}`); },
      error(value) { throw new Error(message(value)); },
      emitFile(file) {
        if (development || file.type !== 'asset') throw new Error('only build assets can be emitted');
        const ref = `rush-${assets.length}`;
        assets.push({ ...file, ref });
        return ref;
      },
      getFileName(ref) { return filename(assetFor(ref)); },
      setAssetSource(ref, source) { assetFor(ref).source = source; },
    };
    try {
      return await (plugin[name].handler ?? plugin[name]).apply(context, args);
    } catch (error) {
      throw new Error(`[${plugin.name}] ${name}: ${message(error)}`, { cause: error });
    }
  }

  async function run(name, args) {
    for (const plugin of hooks(name)) await invoke(plugin, name, args);
  }

  async function resolve(source, importer, options = {}, skip, callData) {
    for (const plugin of hooks('resolveId')) {
      if (plugin === skip) continue;
      const result = await invoke(plugin, 'resolveId', [source, importer ?? undefined, {
        isEntry: false, ...options, ssr,
      }], callData);
      if (typeof result === 'string') return { id: result };
      if (result === false) return { id: source, external: true };
      if (result != null) return result;
    }
    return null;
  }

  function sourceResult(result) {
    if (typeof result === 'string') return { code: result };
    if (typeof result?.code === 'string') return result;
    throw new Error('expected a string or { code, map? }');
  }

  async function load(id, callData) {
    for (const plugin of hooks('load')) {
      const result = await invoke(plugin, 'load', [id, { ssr }], callData);
      if (result != null) return sourceResult(result);
    }
    return null;
  }

  async function transform(source, id, initial = null, callData) {
    let result = initial;
    for (const plugin of hooks('transform')) {
      const output = await invoke(plugin, 'transform', [result?.code ?? source, id, { ssr }], callData);
      if (output != null) result = sourceResult(output);
    }
    return result;
  }

  for (const plugin of hooks('config')) {
    const extra = await invoke(plugin, 'config', [config, env]);
    config = mergePluginConfig(config, extra);
  }
  config = { ...config, root, command, mode, isProduction: !development, configFile, plugins };
  await run('configResolved', [config]);

  async function configureServer(info) {
    await run('configureServer', [{
      config,
      middlewares: {
        use(route, handler) {
          if (typeof route === 'function') [route, handler] = ['/', route];
          if (typeof route !== 'string' || typeof handler !== 'function') {
            throw new Error('expected middlewares.use([path,] handler)');
          }
          middlewares.push([route.replace(/\/$/, '') || '/', handler]);
        },
      },
      resolvedUrls: null,
      ...info,
    }]);
    return middlewares.length > 0;
  }

  async function middleware({ url, method, headers }) {
    for (const [route, handler] of middlewares) {
      const pathname = url.split('?')[0];
      if (route !== '/' && pathname !== route && !pathname.startsWith(route + '/')) continue;
      const request = { url: route === '/' ? url : url.slice(route.length).replace(/^(?!\/)/, '/'),
        originalUrl: url, method, headers };
      const output = {};
      const chunks = [];
      const response = {
        statusCode: 200,
        setHeader(name, value) { output[name.toLowerCase()] = String(value); },
        getHeader(name) { return output[name.toLowerCase()]; },
        removeHeader(name) { delete output[name.toLowerCase()]; },
        writeHead(code, headers = {}) {
          this.statusCode = code;
          for (const [name, value] of Object.entries(headers)) this.setHeader(name, value);
          return this;
        },
        write(chunk) { chunks.push(Buffer.from(chunk)); return true; },
      };
      const handled = await new Promise((done, fail) => {
        response.end = chunk => {
          if (chunk != null) response.write(chunk);
          done(true);
        };
        Promise.resolve(handler(request, response, error => error ? fail(error) : done(false))).catch(fail);
      });
      if (handled) return { status: response.statusCode, headers: output, body: [...Buffer.concat(chunks)] };
    }
    return null;
  }

  async function writeAssets(outdir) {
    for (const asset of assets) {
      const target = pluginPath.resolve(outdir, filename(asset));
      const relative = pluginPath.relative(outdir, target);
      if (!relative || relative === '..' || relative.startsWith('..' + pluginPath.sep) ||
        pluginPath.isAbsolute(relative)) throw new Error(`invalid emitted asset filename: ${filename(asset)}`);
      await pluginFs.mkdir(pluginPath.dirname(target), { recursive: true });
      await pluginFs.writeFile(target, asset.source, { flag: 'wx' });
    }
  }

  return {
    resolve, load, transform, configureServer, middleware, writeAssets,
    hasTransforms: hooks('transform').length > 0,
    useResolver(resolver) { fallbackResolve = resolver; },
    buildStart: () => run('buildStart', [{}]),
    buildEnd: error => run('buildEnd', [error]),
  };
}

function splitPluginId(id) {
  const index = id.search(/[?#]/);
  return index === -1 ? [id, ''] : [id.slice(0, index), id.slice(index)];
}
