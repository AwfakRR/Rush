const readline = require('node:readline');

async function servePlugins() {
  const [configFile, port, https] = process.argv.slice(1);
  const runner = await loadRushPlugins(process.cwd(), configFile, 'serve');
  const middleware = await runner.configureServer({ port: Number(port), https: https === 'true' });
  await runner.buildStart();
  const send = result => process.stdout.write(JSON.stringify(result) + '\n');
  send({ result: middleware });
  const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  try {
    for await (const line of lines) {
      try {
        const { operation, args } = JSON.parse(line);
        let result;
        if (operation === 'imports') {
          const [specifiers, importer] = args;
          result = [];
          for (const source of specifiers) result.push(await runner.resolve(source, importer));
        } else if (operation === 'module') {
          const [id, diskFile] = args;
          const loaded = await runner.load(id);
          const code = loaded?.code ?? (diskFile ? await pluginFs.readFile(diskFile, 'utf8') : null);
          result = code === null ? null : await runner.transform(code, id, loaded);
        } else {
          result = await runner.middleware(args);
        }
        send({ result });
      } catch (error) {
        send({ error: error.message });
      }
    }
  } finally {
    await runner.buildEnd();
  }
}

servePlugins().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
