# Differential test

`tests/differential.rs` generates TypeScript programs, runs each as JavaScript in Node and as the compiler's Luau in
Lune, and fails when the two print different traces. `known.md` lists the differences it has found, and `known/` holds
the smallest program of each one that is still open.

## Running it

It needs `node` (22.18, 23.6 or later, which strip types without a flag) and `lune` on PATH, so it is ignored by default and runs with the
other ignored tests:

```sh
cargo test -p crucible-compiler --test differential -- --ignored
```

| Variable | Default | Meaning |
| --- | --- | --- |
| `DIFFERENTIAL_CASES` | 1000 | Programs to generate. CI runs the default, about two and a half minutes in a debug build. |
| `DIFFERENTIAL_SEED` | fixed | Seed of the generator, so every run without it checks the same programs. |
| `DIFFERENTIAL_BATCH` | 250 | Programs per compile, Node process and Lune process. |
| `DIFFERENTIAL_SHRINK_SECONDS` | 300 | The most time to shrink a failing program. Most shrink in under a minute. |
| `DIFFERENTIAL_KEEP_GOING` | unset | Run every case, write each failing program to `target/tmp/differential/failures`, and shrink none. |
| `DIFFERENTIAL_KEEP` | unset | Keep each batch's folder under `target/tmp/differential`, with its programs and their Luau. |
| `DIFFERENTIAL_FILE` | unset | The `.ts` file the `one_program` test runs, printing both traces line by line. |

A large run is faster in release: `DIFFERENTIAL_CASES=50000 cargo test --release -p crucible-compiler --test
differential -- --ignored generated`. On failure the test prints the seed, the program's number, the first differing
trace line with both values, and the shrunk program, which it also writes to
`target/tmp/differential/failures/seed-<seed>-program-<n>.ts`, with the program as generated next to it in
`seed-<seed>-program-<n>-unshrunk.ts`, so `DIFFERENTIAL_FILE=<either file>` reruns it alone.
`regressions/` holds the programs of fixed bugs, and of constructs the generator does not write where a compiler
change once went unnoticed, such as a computed key with an effect in `||=` and `+=`.
`regressions_trace_the_same_in_node_and_lune` runs each one.
`known_differences_still_differ` runs each program in `known/` and fails when one no longer differs, so a fix moves its
program to `regressions/`.

## How a program runs

The generator builds a typed syntax tree with `proptest`, and `print.rs` writes it as TypeScript, with only the
parentheses its arithmetic, `??`, `&&` and `||` need, so the compiler sees chains such as `a ?? b ?? c` and `a - b + c`
as TypeScript parses them. `shrink.rs` shrinks a failing program in rounds: each round tries every program one edit
away in one batch, where an edit deletes a statement, replaces one with a block of its body, or replaces an expression
with one of its operands, and keeps what still fails the same way. An operand replaces an expression only when it has
the same type, or is an integer where a number was, and keeps what the generator built into an integer's sign, such as
`abs(a)` in `abs(a) % b`. The printer's casts would let TypeScript accept an operand of another type, which can differ
for a reason of its own. A kept program fails exactly as the original did: when the traces differ, they first differ
in the same pair of lines. `shrinking_keeps_the_difference_it_started_from` checks this: it stands an edit of the
program the compiler gets in for the compiler bug `??` lowered to `or`, and requires the shrunk program to keep its `??`
and to agree on the real compiler. A batch is compiled once with `crucible_compiler::compile` in package mode, in a child process that
re-runs the test binary, because tsrs keeps every program in memory until the process exits. A program the compiler
rejects is taken out of the batch and reported as a failure, because the generator should only write programs the
compiler accepts.

- Node runs each program's `.ts` file with its own type stripping, as an ES module. Nothing of roblox-ts or Crucible
  takes part. `node.mjs` adds the methods compiler-types declares and JavaScript lacks (`size`, `isEmpty`, `clear`,
  `upper`, `lower`, `rep`, `insert`, `remove`, `unorderedRemove`, `idiv`, and `size()` and `isEmpty()` on maps and
  sets) as non-enumerable methods.
- Lune runs the compiler's Luau with the vendored `RuntimeLib.lua` and `Promise.lua`, loaded as roblox-ts loads them.
  `lune.luau` gives each program its own environment.

Both get `trace(value, shape)`. The shape is the static type of the value, which the generator knows, because a Luau
value alone cannot tell an empty array from an empty object or `undefined` from a missing key. Each process prints all
of its output at the end, so a program that hangs or crashes its process shows up as missing.

## The trace format

Each program prints `#<name>`, then one line per `trace` call. Both sides print a value the same way:

| Shape | Value | Printed as |
| --- | --- | --- |
| `n` | number | `NaN`, `Infinity`, `-Infinity`, `-0` and `0` by name. An integer below 2^53 in magnitude in full, as `42`. Any other number in the fewest digits that read back as the same number, as `d.ddde<exponent>`: `1.5e0`, `3.0000000000000004e-1`, `1e21`. |
| `s` | string | In double quotes. Printable ASCII other than `"` and `\` as itself, other bytes as `\xHH`. Node writes a character above U+007F as `\u{...}`, and Lune writes its bytes, which never matches, but the generator only writes ASCII. |
| `b` | boolean | `true` or `false`. |
| `?T` | optional | `undefined` (`nil`), or the value as `T`. |
| `[T]` | array | `[a,b,c]`, each element as `T`. Lune also reports a key that is not part of the sequence. |
| `{f:T,...}` | object | `{f=a,...}`, the fields in the order the shape names them, each as its `T`, so a missing field prints `undefined`. |
| `M(K,V)` | map | `M{k=>v,...}`, the entries sorted by their printed text, because the iteration order of a Luau table is not the insertion order. |
| `S(T)` | set | `S{a,b,...}`, sorted the same way. Lune also reports a member whose value is not `true`. |
| `x` | anything | A number, string, boolean or `undefined` as above, `null` as `null`, anything else as `<object>`. |

A value of the wrong kind prints as `<not a number: ...>` and the like. A program that throws prints `!throw <value>`
as its last line. On the Node side an engine error, such as a `TypeError`, prints `!error "<name>: <message>"` and
fails the test as a fault of the generator, because no generated program should cause one. Lune prints `!syntax
"<message>"` when the Luau does not load.

Integers and other numbers print differently so that a difference between `1` and `1.0000000000000002` reads at once.
The shortest round-trip digits are what JavaScript's `String` picks, and Lune finds them by trying `%.<p>e` from 1 to
17 digits. An earlier fixed 17-digit format failed on numbers whose 17th digit is a tie, because JavaScript's
`toExponential` rounds half up and C rounds half to even.

## What the generator covers

Types: integers, other numbers, strings, booleans, optional numbers, strings and booleans, arrays of each and of objects, an
object type, a record type whose field names are Luau keywords (`end`, `then`), optional objects, `Map<string,
number>`, `Map<number, string>`, `Set<string>`, `Set<number>`, two function types, and class instances.

- Expressions: number, string, boolean, array, object, map and set literals with spreads, including numbers in hex,
  octal, binary and with separators and strings with escapes and Luau syntax inside; arithmetic, `**`, unary `-` and
  `+`, `%` over operand expressions in any position, `idiv` (Luau's `//`), bitwise and shift operators; `===`, `!==`, `<`, `<=`, `>`, `>=` on numbers, strings and booleans; `!`, `&&`
  and `||` on every primitive, `??` and `||` and `&&` over optional booleans and over defined `false`, `0`, `""` and NaN, `??`
  chains, `&&`, `||` and `??` as statements, `?:`, optional chains, optional method calls and calls of an optional
  function, `=== undefined`, `===` of values of different types, `!`, `as` and `satisfies`; template strings and `+` with
  strings, numbers and booleans; `++`, `--`, `=` and `+=` as values; `in`, `instanceof`, `delete`; calls of top-level
  functions with default and rest parameters and spread arguments, of function variables and of closures written in
  place; array methods `size`, `isEmpty`, `push`, `pop`, `shift`, `indexOf`, `includes`, `find`, `findIndex`, `some`,
  `every`, `map`, `filter`, `reduce` with and without an initial value, `forEach`, `join`, and roblox-ts's `insert`,
  `remove` and `unorderedRemove`; string methods `size`,
  `upper`, `lower`, `rep`, `split`; map and set methods `get`, `set`, `has`, `add`, `delete`, `size`, `isEmpty`;
  element reads at any index, such as `xs[i + 1]`, `xs[i - 1]`, `xs[xs.size() - 1]` and `xs[j * 2]`, in bounds and
  past either end, where both sides read `undefined`, and optional element reads `xs?.[i]`, whose index runs only
  when the array is there.
- Statements: `let` and `const`, shadowing in inner blocks, compound assignment (`+=` on strings with numbers and
  booleans, `-=`, `*=`, `/=`, `**=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`, `>>>=`) and logical assignment (`??=`, `||=`, `&&=`) on variables, fields
  and elements, `if`, counting `for` loops up and down, loops over an array's indices up and down that read past either end and
  remove every second or third element as they go, a closure over a `for` loop's counter called after the loop, `for...of` over arrays, strings, maps and sets, with an object pattern,
  `forEach`, bounded `while` and `do...while`, with a condition that needs statements and one that names a variable
  the body hides, `break`, `continue`, `switch`
  with fallthrough, `default` and cases that are expressions, `try` with `catch` and
  `finally` and `return`, `break` or `continue` inside them, `throw` of strings and integers, array and object
  destructuring with holes, defaults, rest and nesting, destructuring assignment, swaps, blocks, and functions declared
  inside a block.
- Mutation of arrays, maps, sets, object fields and record fields through variables, including inside closures and
  callbacks. Writes to an element take any index that reads only variables and lengths, under a bounds check.
- Top-level functions, recursion, hoisting (functions printed after the code that calls them), and classes with fields,
  field initializers, constructors, methods, `this`, static fields and methods, subclasses with `super(...)`,
  `super.m(...)` and overrides.

Programs terminate and are deterministic: loops have constant bounds, a guard counter, or an array's length, which the body may not change, the only recursion counts
down, and nothing reads time, randomness or iteration order of a map or set (a loop over one adds an order-free term
to a sum). A string is written only while it is shorter than 256 characters, so a loop that doubles one stays under
Node's limit on the length of a string.

## What it leaves out

Each open entry of `known.md` is left out where the generator would write it, with a comment that names the entry.

| Left out | Why |
| --- | --- |
| Several arguments of `push` or `unshift` with an effect, other than the one evaluated first | known.md, "push and unshift insert a value before they evaluate the next". |
| A negative `fromIndex` of `indexOf` and `includes` | known.md, "A negative fromIndex throws". |
| `includes` on an array that may hold NaN | known.md, "includes(NaN) is false". |
| `join()` with no separator | known.md, "join() separates with a comma and a space". |
| `toString` and an instance turned into a string | known.md, "An inherited toString is lost". |
| `%` of operands of opposite signs | known.md, "% is floored". The generator builds each operand of `%` so the signs agree: one not below 0 by one above it, or two below 0, followed by `+ 0`, because JavaScript then gives `-0` for a zero remainder and Luau `0`. |
| `**` by any exponent but 0, 1, 2 and -1 | known.md, "** is C's pow". |
| A number other than an integer turned into a string | known.md, "A number becomes a string through tostring". |
| Strings that are not ASCII | known.md, "A string's size counts bytes", and `split`, `upper` and `lower` work on bytes too. |
| Truthiness of numbers and strings without the lint suppressed | CRU032 rejects it, because `0`, `NaN` and `""` are truthy in Luau. The generator writes it with `// oxlint-disable-next-line roblox-ts/lua-truthiness`, so the lowered `&&`, `\|\|`, `!` and conditions are checked. |
| `null`, `typeof`, `==` and `!=` | The compiler rejects them, as roblox-ts does. |
| Arrays that hold `undefined` | Luau's `#` may return any border of a table with holes. roblox-ts documents it. |
| `sort` | Luau's `table.sort` is not stable and takes a boolean comparator, so the two are different functions. |
| Getters, setters, private names, labels, generators, `async`, `await`, regular expressions, bigints, tagged templates, class expressions, object rest | The compiler rejects them. |
| Writing an array element past its end | JavaScript leaves a hole, and Luau's `#` may then return any border. A write checks its index, and may append at the end. |
| Iteration order of maps and sets | A Luau table has no insertion order, which roblox-ts documents. Traces sort map and set entries, and loops sum order-free terms. |
| Negative operands of bitwise operators, `~`, and shifts past 8 | known.md, "Bitwise results are unsigned". |
| Errors thrown by the engine, such as calling `undefined` | Both sides fail, with messages that differ by design. Node reports one as a fault of the generator. |
| Generic functions, interfaces, `import` and `export` | Out of scope for one-file programs of this test; each needs its own runner support. |
| Enums and namespaces | Node only strips types, and both need a transform, so they would need a TypeScript compiler on the Node side. |
| String and math functions of `@rbxts/types`, such as `sub`, `find` and `math.floor` | JavaScript lacks them, so the Node side would run a copy of Luau's library written for the test, which tests that copy more than the compiler. |
