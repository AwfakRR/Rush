import React from 'react';

const h = React.createElement;

export function App() {
  const [count, setCount] = React.useState(0);
  const [ready, setReady] = React.useState(false);
  React.useEffect(() => setReady(true), []);

  return h('main', null,
    h('p', { className: 'eyebrow' }, 'RUSH / REACT SSR'),
    h('h1', null, 'From server to screen.'),
    h('p', { className: 'description' },
      'This page arrived as HTML. React then hydrated it in the browser, without replacing the markup.'),
    h('button', { onClick: () => setCount(count + 1) }, `Count: ${count}`),
    h('p', { className: 'status', 'aria-live': 'polite' },
      ready ? '● Hydrated in the browser' : '○ Rendered on the server'),
  );
}
