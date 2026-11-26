function frameworkPackage(name) {
  return dependency(name, 'Framework compilation');
}

function digest(value) {
  return createHash('sha256').update(value).digest('hex').slice(0, 12);
}

function componentId(file) {
  return digest(path.relative(root, file).split(path.sep).join('/'));
}

function checkCompilerErrors(errors) {
  if (errors.length) throw new Error(errors.map(error => error.message || error).join('\n'));
}

function compileVue(file, source, production) {
  const vue = frameworkPackage('@vue/compiler-sfc');
  const { descriptor, errors } = vue.parse(source, { filename: file });
  checkCompilerErrors(errors);
  const id = componentId(file);
  const scope = 'data-v-' + id;
  const scoped = descriptor.styles.some(style => style.scoped);
  if ([descriptor.script, descriptor.scriptSetup, descriptor.template, ...descriptor.styles]
    .some(block => block?.src)) {
    throw new Error('Vue SFCs with external src blocks are not supported.');
  }
  const script = descriptor.script || descriptor.scriptSetup
    ? vue.compileScript(descriptor, { id, isProd: production, genDefaultAs: '__rush_component' })
    : null;
  let code = script?.content ?? 'const __rush_component = {};';
  const scriptKey = digest(code + String(scoped));
  if (descriptor.template) {
    const template = vue.compileTemplate({
      source: descriptor.template.content,
      filename: file,
      id,
      scoped,
      slotted: descriptor.slotted,
      isProd: production,
      compilerOptions: { bindingMetadata: script?.bindings },
    });
    checkCompilerErrors(template.errors);
    code += `\n${template.code}\n__rush_component.render = render;`;
  }
  if (scoped) code += `\n__rush_component.__scopeId = '${scope}';`;
  code += '\nexport default __rush_component;\n';
  if (!production) {
    code += `
export const __rush_vue_script = '${scriptKey}';
__rush_component.__hmrId = '${id}';
const __rush_vue_hmr = globalThis.__VUE_HMR_RUNTIME__;
__rush_vue_hmr?.createRecord('${id}', __rush_component);
import.meta.hot.accept((module) => {
  if (module.__rush_vue_script === __rush_vue_script) {
    __rush_vue_hmr?.rerender('${id}', module.default.render);
  } else {
    __rush_vue_hmr?.reload('${id}', module.default);
  }
});
`;
  }
  const css = descriptor.styles.map(style => {
    const result = vue.compileStyle({
      source: style.content, filename: file, id: scope, scoped: style.scoped, isProd: production,
    });
    checkCompilerErrors(result.errors);
    return result.code;
  }).join('\n');
  const loader = [descriptor.script?.lang, descriptor.scriptSetup?.lang].includes('ts') ? 'ts' : 'js';
  return { code, css, loader };
}

function compileSvelte(file, source, production) {
  const svelte = frameworkPackage('svelte/compiler');
  const result = svelte.compile(source, {
    filename: path.relative(root, file),
    generate: 'client',
    dev: !production,
    hmr: !production,
  });
  return { code: result.js.code, css: result.css?.code ?? '', loader: 'js' };
}

async function compileSolid(file, source, production) {
  if (file.endsWith('.tsx')) {
    source = (await frameworkPackage('esbuild').transform(source, { loader: 'tsx', jsx: 'preserve', sourcefile: file })).code;
  }
  const babel = frameworkPackage('@babel/core');
  const result = await babel.transformAsync(source, {
    filename: file,
    babelrc: false,
    configFile: false,
    presets: [[frameworkPackage('babel-preset-solid'), { generate: 'dom' }]],
    plugins: production ? [] : [[frameworkPackage('solid-refresh/babel'), { bundler: 'vite' }]],
  });
  return { code: result.code, css: '', loader: 'js' };
}

function solidProject() {
  return process.env.RUSH_SOLID === 'true';
}

function frameworkFile(file) {
  return /\.(vue|svelte)$/.test(file) || (solidProject() && /\.[jt]sx$/.test(file));
}

async function compileFramework(file, production) {
  const source = await fs.readFile(file, 'utf8');
  if (file.endsWith('.vue')) return compileVue(file, source, production);
  if (file.endsWith('.svelte')) return compileSvelte(file, source, production);
  return compileSolid(file, source, production);
}

async function devComponent(file) {
  let { code, css, loader } = await compileFramework(file, false);
  if (loader === 'ts') {
    code = (await frameworkPackage('esbuild').transform(code, { loader, sourcefile: file })).code;
  }
  if (/\.(vue|svelte)$/.test(file)) {
    const id = componentId(file);
    code += `
import.meta.hot.data.rushStyleRevision = '${digest(code)}';
const __rush_style = document.querySelector('style[data-rush-sfc="${id}"]') || document.createElement('style');
__rush_style.dataset.rushSfc = '${id}';
__rush_style.textContent = ${JSON.stringify(css)};
if (!__rush_style.isConnected) document.head.appendChild(__rush_style);
`;
  }
  code += '\nimport.meta.hot.data.rushFramework = true;';
  return { code };
}

function frameworkPlugin() {
  const styles = new Map();
  return {
    name: 'rush-frameworks',
    setup(build) {
      build.onResolve({ filter: /^\/.*\.(vue|svelte)$/ }, async (args) => {
        if (args.kind === 'entry-point' || args.pluginData?.rushComponentResolved) return;
        return build.resolve(path.join(root, args.path), {
          importer: args.importer,
          kind: args.kind,
          resolveDir: args.resolveDir,
          pluginData: { rushComponentResolved: true },
        });
      });
      build.onResolve({ filter: /^rush-component-style:/ }, ({ importer }) => ({
        path: importer,
        namespace: 'rush-component-style',
      }));
      build.onLoad({ filter: /.*/, namespace: 'rush-component-style' }, ({ path: file }) => ({
        contents: styles.get(file),
        loader: 'css',
        resolveDir: path.dirname(file),
      }));
      build.onLoad({ filter: /\.(vue|svelte|jsx|tsx)$/, namespace: 'file' }, async ({ path: file }) => {
        if (!frameworkFile(file)) return;
        let { code, css, loader } = await compileFramework(file, true);
        if (css) {
          styles.set(file, css);
          code += '\nimport "rush-component-style:css";';
        }
        return { contents: code, loader, resolveDir: path.dirname(file) };
      });
    },
  };
}

async function frameworkDeps(entries, outdir) {
  const esbuild = frameworkPackage('esbuild');
  await esbuild.build({
    entryPoints: entries.map(specifier => ({ in: specifier, out: specifier.replaceAll('/', '__') })),
    absWorkingDir: root,
    outdir,
    bundle: true,
    splitting: true,
    format: 'esm',
    platform: 'browser',
    mainFields: ['browser', 'module', 'main'],
    conditions: ['development', 'module'],
    define: { 'process.env.NODE_ENV': '"development"' },
    target: 'es2020',
    sourcemap: true,
  });
  return {};
}
