export interface CounterState {
  count: number;
}

export function createCounter(initial: number = 0): { increment: () => number; get: () => number } {
  let count: number = initial;
  return {
    increment: (): number => ++count,
    get: (): number => count,
  };
}
