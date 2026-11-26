const fs = require('node:fs/promises');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { createRequire } = require('node:module');

const root = process.cwd();
const projectRequire = createRequire(path.join(root, 'package.json'));
console.log = console.info = console.debug = console.error;

function dependency(name) {
  let file;
  try {
    file = projectRequire.resolve(name);
  } catch (error) {
    if (error.code !== 'MODULE_NOT_FOUND') throw error;
    throw new Error(`CSS processing requires ${name}. Run npm install -D ${name} in ${root}.`);
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

async function build(entry, outdir, configured) {
  const esbuild = dependency('esbuild');
  const processCss = await processor(configured);
  const result = await esbuild.build({
    entryPoints: [entry],
    outdir,
    bundle: true,
    minify: true,
    format: 'esm',
    sourcemap: true,
    target: 'es2020',
    entryNames: '[name]-[hash]',
    assetNames: '[name]-[hash]',
    metafile: true,
    plugins: [{
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
            contents: `import ${JSON.stringify('rush-css:' + file)};\n${exports}`,
            loader: 'js',
          };
        });
        const loadStyle = async ({ path: file }) => ({
          contents: (await processCss(file)).css,
          loader: 'css',
          resolveDir: path.dirname(file),
        });
        build.onLoad({ filter: /\.css$/, namespace: 'file' }, loadStyle);
        build.onLoad({ filter: /.*/, namespace: 'rush-css-style' }, loadStyle);
      },
    }],
  });
  return result.metafile;
}

async function main() {
  const [action, configured, file, outdir] = process.argv.slice(1);
  const output = action === 'build'
    ? await build(file, outdir, configured === 'true')
    : await (await processor(configured === 'true'))(file);
  process.stdout.write(JSON.stringify(output));
}

main().catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});
