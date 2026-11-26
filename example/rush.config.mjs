import replace from '@rollup/plugin-replace';
import virtual from '@rollup/plugin-virtual';
import yaml from '@rollup/plugin-yaml';

export default ({ mode }) => ({
  plugins: [
    virtual({ 'virtual:welcome': 'export default "Hello from a virtual module";' }),
    yaml(),
    replace({ preventAssignment: true, values: { __DEMO_MODE__: JSON.stringify(mode) } }),
  ],
});
