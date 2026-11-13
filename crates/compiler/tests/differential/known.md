# Known differences

What the differential test found where a program's Luau does not do what its JavaScript does. Each entry has a minimal
program, run through the test's own runners with `DIFFERENTIAL_FILE=<file> cargo test -p crucible-compiler --test
differential -- --ignored one_program --nocapture`, and what each side printed. The generator leaves out exactly the
construct of each open entry, with a comment that names the entry's title.

roblox-ts 3.0.0 below means `roblox-ts/out/TSTransformer` of the published package, which the compiler follows.

## Fixed

### A call statement in parentheses opened a block with `;`

`crucible-luau` printed `;` before a statement that starts with `(`, so Luau does not read it as a call of the line
before. It did so at the start of a block too, where Luau rejects a `;` with nothing before it, so the whole module
failed to load. The printer now leaves the `;` out on a block's first statement. The program is
`regressions/call-of-parenthesized-callee-opens-a-block.ts`.

```ts
for (let i = 0; i < 2; i++) {
	((n: number): number => n)(i);
}
```

- Node: runs.
- Lune, before the fix: `!syntax "...: Expected identifier when parsing expression, got ';'"`.

## Compiler, unlike roblox-ts

### An inherited toString is lost

A class gets a `__tostring` metamethod only when it declares `toString` itself. Lua does not look metamethods up
through `__index`, so an instance of a subclass that inherits `toString` prints as a table. roblox-ts 3.0.0 asks the
checker whether the instance type has a `toString` method, inherited ones included
(`transformClassLikeDeclaration.js`, `instanceType.getProperty(MAGIC_TO_STRING_METHOD)`), and emits `__tostring` on the
subclass too. `lower/class.rs` only reads the class's own members. The fix needs a fact from the checker, recorded in
extract and read when the class lowers, which is more than this pull request may change.

```ts
class Base {
	toString(): string {
		return "base";
	}
}
class Derived extends Base {}
trace(`${new Derived()}`, "s");
```

- Node: `"base"`.
- Lune: `"table: 0x..."`.

The generator never declares `toString` and never turns an instance into a string.

## Compiler and roblox-ts alike

The compiler lowers these the way roblox-ts 3.0.0 does, and JavaScript differs. A fix parts from roblox-ts, and
changes the output of code that compiles today.

### push and unshift insert a value before they evaluate the next

`push` and `unshift` with several arguments lower to one `table.insert` per argument, each with its argument inside,
so an argument evaluated later sees the values inserted before it. `unshift` inserts from the last argument, so it
also evaluates its arguments from last to first. JavaScript evaluates every argument first, from left to right.
roblox-ts 3.0.0 does the same (`propertyCallMacros.js`, `push` and `unshift`). Evaluating each argument into a
temporary first would change the runtime's own output, as `internal/configs/runtime.ts` pushes two call results at
once, so it is not fixed here.

```ts
function look(xs: number[]): number {
	trace(xs, "[n]");
	return 9;
}
const xs: number[] = [];
xs.push(0, look(xs));
function say(n: number): number {
	trace(n, "n");
	return n;
}
const ys: number[] = [];
ys.unshift(say(1), say(2));
```

- Node: `[]`, then `1`, `2`.
- Lune: `[0]`, then `2`, `1`.

The generator gives `push` and `unshift` several arguments only when all but the one evaluated first are literals or
variables.

### A negative fromIndex throws

`indexOf` and `includes` pass `fromIndex + 1` to `table.find`, which rejects an index below 1 instead of counting from
the end, as JavaScript does.

```ts
const xs: number[] = [1, 2, 3];
trace(xs.indexOf(3, -1), "n");
trace(xs.includes(1, -1), "b");
```

- Node: `2`, then `false`.
- Lune: `!throw "[string \x22p00000\x22]:3: invalid argument #3 to 'find' (index out of range)"`, and nothing after.

The generator draws `fromIndex` from 0 to 3.

### includes(NaN) is false

`table.find` compares with `==`, and `NaN == NaN` is false. JavaScript's `includes` uses SameValueZero, which finds
NaN. `indexOf` agrees on both sides, because JavaScript's `indexOf` uses `===` too.

```ts
trace([0 / 0].includes(0 / 0), "b");
```

- Node: `true`.
- Lune: `false`.

The generator calls `includes` on `number[]` only for arrays of integers, which never hold NaN.

### join() separates with a comma and a space

With no separator, `join` passes `", "` to `table.concat`. compiler-types documents this, and roblox-ts does the same.
JavaScript separates with `","`.

```ts
trace([1, 2, 3].join(), "s");
```

- Node: `"1,2,3"`.
- Lune: `"1, 2, 3"`.

The generator always passes a separator.

## Luau semantics that roblox-ts keeps

roblox-ts lowers these operators and conversions to Luau's own, on purpose. The generator leaves out the inputs where
the two languages part.

### % is floored

`%` is Luau's `%`, which takes the sign of the divisor. JavaScript's takes the sign of the dividend.

```ts
trace(id(-7) % 3, "n");
trace(id(7) % -3, "n");
trace(id(-0) % 3, "n");
```

- Node: `-1`, `1`, `-0`.
- Lune: `2`, `-2`, `0`.

The generator takes `%` of `abs(x)` by a positive constant.

### ** is C's pow

`**` is Luau's `^`. Luau computes `x ^ 0.5` as a square root, which keeps the sign of `-0`, and `x ^ 3` with a
constant 3 as `x * x * x`, which can be one unit in the last place from `pow`. C's `pow` returns 1 for `1 ^ NaN` and
`(-1) ^ ±inf`, where JavaScript returns NaN. A constant `(-0) ^ 0.5` folds to `0` at compile time and agrees. Other
exponents go to two different `pow` implementations, V8's and the C library's, which ECMAScript lets differ in the last
place.

```ts
trace(id(-0) ** 0.5, "n");
trace(id(-0.30000000000000004) ** 3, "n");
trace(id(1) ** (0 / 0), "n");
trace(id(-1) ** (1 / 0), "n");
```

- Node: `0`, `-2.7000000000000014e-2`, `NaN`, `NaN`.
- Lune: `-0`, `-2.700000000000001e-2`, `1`, `1`.

The generator's exponents are 0, 1, 2 and -1, which both sides compute exactly.

### Bitwise results are unsigned

`&`, `|`, `^`, `~`, `<<` and `>>` lower to `bit32`, which returns an unsigned 32-bit integer where JavaScript returns
a signed one, and does not take a shift count modulo 32.

```ts
trace(id(-1) | 0, "n");
trace(id(1) << 31, "n");
trace(id(-8) >> 1, "n");
trace(~id(5), "n");
trace(id(1) << 33, "n");
```

- Node: `-1`, `-2147483648`, `-4`, `-6`, `2`.
- Lune: `4294967295`, `2147483648`, `4294967292`, `4294967290`, `0`.

`>>>` agrees. The generator takes each operand of a bitwise operator modulo 256 after `abs`, shifts by 0 to 8, and
never uses `~`.

### A number becomes a string through tostring

A template or `+` with a string turns a number into a string with Luau's `tostring`. It agrees with JavaScript on
integers and on most other finite numbers, and differs on `-0`, the infinities, NaN, and exponents below -4.

```ts
trace(`${id(-0)}`, "s");
trace(`${id(1 / 0)}`, "s");
trace(`${id(0 / 0)}`, "s");
trace(`${id(1e-7)}`, "s");
```

- Node: `"0"`, `"Infinity"`, `"NaN"`, `"1e-7"`.
- Lune: `"-0"`, `"inf"`, `"nan"`, `"1e-07"`.

The generator turns only integers into strings.

### A string's size counts bytes

`size()` is `#`, which counts UTF-8 bytes. JavaScript's `length` counts UTF-16 code units.

```ts
trace(id("é").size(), "n");
trace(id("😀").size(), "n");
```

- Node: `1`, `2`.
- Lune: `2`, `4`.

The generator's strings are ASCII.
