declare function trace(value: unknown, shape: string): void;
const xs: number[] = [1, 2, 3];
trace(xs.indexOf(3, -1), "n");
