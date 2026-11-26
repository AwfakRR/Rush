import { greet } from "rush-example-greet";
import { add } from "./math";

export { Calculator, add } from "./math";
export type { Operands } from "./math";

export function describeSum(name: string, a: number, b: number): string {
  return `${greet(name)}: ${add(a, b)}`;
}
