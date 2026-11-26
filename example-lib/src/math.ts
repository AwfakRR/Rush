export interface Operands {
  a: number;
  b: number;
}

export function add(a: number, b: number): number {
  return a + b;
}

export class Calculator {
  total({ a, b }: Operands): number {
    return add(a, b);
  }
}
