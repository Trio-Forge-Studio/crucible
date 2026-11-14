// A call statement whose callee is in parentheses was printed after a `;`, which Luau rejects at the start of a block.
declare function trace(value: unknown, shape: string): void;
for (let i = 0; i < 2; i++) {
	((n: number): number => n)(i);
}
if (trace !== undefined) {
	((s: string): void => trace(s, "s"))("ran");
}
trace(1, "n");
