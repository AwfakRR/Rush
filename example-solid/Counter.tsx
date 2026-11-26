import { createSignal } from 'solid-js';

export default function Counter() {
  const [count, setCount] = createSignal<number>(0);

  return (
    <>
      <h1>Count with Solid</h1>
      <p>Edit this component while the counter is running.</p>
      <button type="button" onClick={() => setCount(count() + 1)}>Count: {count()}</button>
    </>
  );
}
