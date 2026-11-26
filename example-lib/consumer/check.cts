import library = require("rush-example-lib");

const operands: library.Operands = { a: 40, b: 2 };
const total: number = new library.Calculator().total(operands);

// @ts-expect-error The library accepts numbers, not numeric strings.
library.add(String(total), 0);
