const fs = require('node:fs/promises');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { createRequire } = require('node:module');

const root = process.cwd();
const projectRequire = createRequire(path.join(root, 'package.json'));

function dependency(name, purpose = 'CSS processing') {
  let file;
  try {
    file = projectRequire.resolve(name);
  } catch (error) {
    if (error.code !== 'MODULE_NOT_FOUND') throw error;
    throw new Error(`${purpose} requires ${name}. Run npm install -D ${name.split('/').slice(0, name.startsWith('@') ? 2 : 1).join('/')} in ${root}.`);
  }
  return projectRequire(file);
}

function classExports(classes) {
  let code = `const classes = JSON.parse(${JSON.stringify(JSON.stringify(classes))});\nexport default classes;\n`;
  let index = 0;
  for (const name of Object.keys(classes)) {
    if (name !== 'default' && /^[$_a-z][$_a-z0-9]*$/i.test(name)) {
      const local = `class${index++}`;
      code += `const ${local} = classes[${JSON.stringify(name)}];\nexport { ${local} as ${name} };\n`;
    }
  }
  return code;
}

async function processor(configured) {
  const config = configured ? await dependency('postcss-load-config')({}, root) : null;
  const cache = new Map();

  async function processCss(file) {
    let css = await fs.readFile(file, 'utf8');
    const isModule = file.endsWith('.module.css');
    if (!config && !isModule) return { css };

    const postcss = dependency('postcss');
    if (config) {
      const result = await postcss(config.plugins).process(css, {
        ...config.options, from: file, to: file, map: false,
      });
      for (const warning of result.warnings()) console.error(warning.toString());
      css = result.css;
    }
    if (!isModule) return { css };

    let classes;
    const modules = dependency('postcss-modules');
    const result = await postcss([modules({
      generateScopedName(name, filename) {
        const relative = path.relative(root, filename).split(path.sep).join('/');
        const hash = createHash('sha256').update(relative).digest('hex').slice(0, 10);
        return `_rush_${hash}_${name}`;
      },
      getJSON(_file, json) { classes = json; },
    })]).process(css, { from: file, map: false });
    return { css: result.css, classes, exports: classExports(classes) };
  }

  return (file) => {
    if (!cache.has(file)) cache.set(file, processCss(file));
    return cache.get(file);
  };
}

function rushPluginBridge(runner) {
  const loaders = { '.js': 'js', '.mjs': 'js', '.cjs': 'js', '.ts': 'ts', '.mts': 'ts',
    '.cts': 'ts', '.jsx': 'jsx', '.tsx': 'tsx' };
  return {
    name: 'rush-vite-plugins',
    setup(build) {
      const resolveDir = importer => importer && path.isAbsolute(importer)
        ? path.dirname(splitPluginId(importer)[0]) : root;
      runner.useResolver(async (source, importer, _options, args) => {
        const result = await build.resolve(source, {
          importer, resolveDir: resolveDir(importer),
          kind: args?.kind ?? 'import-statement', pluginData: { rushSkipPlugins: true },
        });
        return result.errors.length ? null : { id: result.path + result.suffix, external: result.external };
      });
      build.onStart(() => runner.buildStart());
      build.onResolve({ filter: /.*/ }, async args => {
        if (args.pluginData?.rushSkipPlugins || args.pluginData?.rushResolved || args.path.startsWith('rush-css:')) return;
        const result = await runner.resolve(args.path, args.importer, { isEntry: args.kind === 'entry-point' }, args);
        if (!result || args.kind === 'entry-point') return;
        if (result.external) return { path: result.id, external: true };
        const [file, suffix] = splitPluginId(result.id);
        if (path.isAbsolute(file)) return { path: file, suffix };
        return { path: result.id, namespace: 'rush-plugin' };
      });
      const onLoad = async args => {
        const { path: file, suffix, namespace } = args;
        const id = file + suffix;
        const loaded = await runner.load(id, args);
        const extension = path.extname(splitPluginId(file)[0]);
        const nativeJs = loaders[extension];
        let code = loaded?.code;
        if (code === undefined && namespace === 'file' && runner.hasTransforms &&
          (nativeJs || /\.(json|ya?ml|svg|txt|md|vue|svelte)$/.test(file))) {
          code = await fs.readFile(file, 'utf8');
        }
        if (code === undefined) return;
        const result = await runner.transform(code, id, loaded, args);
        if (!result) return;
        const map = typeof result.map === 'string' ? result.map : JSON.stringify(result.map);
        return {
          contents: map && result.map
            ? result.code + '\n//# sourceMappingURL=data:application/json;base64,' + Buffer.from(map).toString('base64')
            : result.code,
          loader: nativeJs ?? 'js',
          resolveDir: resolveDir(id),
        };
      };
      build.onLoad({ filter: /.*/, namespace: 'file' }, onLoad);
      build.onLoad({ filter: /.*/, namespace: 'rush-plugin' }, onLoad);
    },
  };
}

async function build(entries, outdir, configured, configFile, ssr = false) {
  const esbuild = dependency('esbuild', ssr ? 'SSR' : 'building the project');
  const processCss = await processor(configured);
  const command = ssr && process.env.NODE_ENV === 'development' ? 'serve' : 'build';
  const runner = configFile && await loadRushPlugins(root, configFile, command, ssr);
  const options = {
    entryPoints: entries,
    outdir,
    bundle: true,
    splitting: true,
    minify: true,
    format: 'esm',
    sourcemap: true,
    target: 'es2020',
    entryNames: '[name]-[hash]',
    chunkNames: '[name]-[hash]',
    assetNames: '[name]-[hash]',
    metafile: true,
    plugins: [...(runner ? [rushPluginBridge(runner)] : []), ...(ssr ? [] : [frameworkPlugin()]), {
      name: 'rush-css',
      setup(build) {
        build.onResolve({ filter: /^rush-css:/, namespace: 'rush-css-module' }, (args) => ({
          path: args.path.slice('rush-css:'.length),
          namespace: 'rush-css-style',
        }));
        build.onResolve({ filter: /\.module\.css$/ }, async (args) => {
          if (args.pluginData?.rushResolved || !['import-statement', 'dynamic-import', 'require-call'].includes(args.kind)) return;
          const specifier = args.path.startsWith('/') ? path.join(root, args.path) : args.path;
          const resolved = await build.resolve(specifier, {
            importer: args.importer,
            namespace: args.namespace,
            resolveDir: args.resolveDir,
            kind: args.kind,
            pluginData: { rushResolved: true },
          });
          if (resolved.errors.length || resolved.external) return resolved;
          return { path: resolved.path, namespace: 'rush-css-module' };
        });
        build.onLoad({ filter: /.*/, namespace: 'rush-css-module' }, async ({ path: file }) => {
          const { exports } = await processCss(file);
          return {
            contents: ssr ? exports : `import ${JSON.stringify('rush-css:' + file)};\n${exports}`,
            loader: 'js',
          };
        });
        const loadStyle = async ({ path: file }) => ({
          contents: (await processCss(file)).css,
          loader: 'css',
          resolveDir: path.dirname(file),
        });
        build.onLoad({ filter: /\.css$/, namespace: 'file' }, ssr
          ? () => ({ contents: '', loader: 'empty' })
          : loadStyle);
        build.onLoad({ filter: /.*/, namespace: 'rush-css-style' }, loadStyle);
      },
    }],
  };
  if (ssr) {
    Object.assign(options, {
      platform: 'node',
      packages: 'external',
      target: 'node20',
      jsx: 'automatic',
      splitting: false,
      minify: command === 'build',
      sourcemap: 'inline',
      write: false,
      metafile: false,
    });
  }
  let result;
  try {
    result = await esbuild.build(options);
  } catch (error) {
    if (runner) await runner.buildEnd(error);
    throw error;
  }
  if (runner) {
    await runner.buildEnd();
    if (!ssr) await runner.writeAssets(path.dirname(outdir));
  }
  return ssr ? result.outputFiles[0].text : result.metafile;
}

async function main() {
  const [action, configured, file, ...entries] = process.argv.slice(1);
  let output;
  switch (action) {
    case 'build': output = await build(entries, file, configured === 'true'); break;
    case 'plugin-build': output = await build(entries.slice(1), file, configured === 'true', entries[0]); break;
    case 'ssr': output = await build([file], undefined, configured === 'true', entries[0], true); break;
    case 'component': output = await devComponent(file); break;
    case 'scan-components': {
      output = [];
      for (const entry of [file, ...entries]) {
        try {
          output.push(await compileFramework(entry, false));
        } catch (error) {
          console.error(`${entry}: ${error.message}`);
        }
      }
      break;
    }
    case 'framework-deps': output = await frameworkDeps(entries, file); break;
    default: output = await (await processor(configured === 'true'))(file);
  }
  process.stdout.write(JSON.stringify(output));
}

main().catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});
