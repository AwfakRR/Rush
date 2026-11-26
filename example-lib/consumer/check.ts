import { Calculator, describeSum, type Operands } from "rush-example-lib";

const args: Operands = { a: 40, b: 2 };
const result: number = new Calculator().total(args);
const description: string = describeSum("TypeScript", result, 0);

// @ts-expect-error The library accepts numbers, not numeric strings.
describeSum(description, "40", 2);
