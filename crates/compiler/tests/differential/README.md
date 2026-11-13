# Differential test

`tests/differential.rs` generates TypeScript programs, runs each as JavaScript in Node and as the compiler's Luau in
Lune, and fails when the two print different traces. `known.md` lists the differences it has found.

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
| `DIFFERENTIAL_SHRINK_SECONDS` | 300 | How long to shrink a failing program. |
| `DIFFERENTIAL_KEEP_GOING` | unset | Run every case, write each failing program to `target/tmp/differential/failures`, and shrink none. |
| `DIFFERENTIAL_FILE` | unset | The `.ts` file the `one_program` test runs, printing both traces line by line. |

A large run is faster in release: `DIFFERENTIAL_CASES=50000 cargo test --release -p crucible-compiler --test
differential -- --ignored generated`. On failure the test prints the first differing trace line with both values and
the shrunk program, which is a standalone `.ts` file. `regressions/` holds the programs of fixed bugs, and
`regressions_trace_the_same_in_node_and_lune` runs each one.

## How a program runs

The generator builds a typed syntax tree with `proptest`, so a failing program shrinks, and `print.rs` writes it as
TypeScript. A batch is compiled once with `crucible_compiler::compile` in package mode, in a child process that
re-runs the test binary, because tsrs keeps every program in memory until the process exits. A program the compiler
rejects is taken out of the batch and reported as a failure, because the generator should only write programs the
compiler accepts.

- Node runs each program's `.ts` file with its own type stripping, as an ES module. Nothing of roblox-ts or Crucible
  takes part. `node.mjs` adds the methods compiler-types declares and JavaScript lacks (`size`, `isEmpty`, `clear`,
  `upper`, `lower`, `rep`, and `size()` and `isEmpty()` on maps and sets) as non-enumerable methods.
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

Types: integers, other numbers, strings, booleans, optional numbers and strings, arrays of each and of objects, an
object type, a record type whose field names are Luau keywords (`end`, `then`), optional objects, `Map<string,
number>`, `Map<number, string>`, `Set<string>`, `Set<number>`, two function types, and class instances.

- Expressions: number, string, boolean, array, object, map and set literals with spreads, including numbers in hex,
  octal, binary and with separators and strings with escapes and Luau syntax inside; arithmetic, `**`, unary `-` and
  `+`, `%`, bitwise and shift operators; `===`, `!==`, `<`, `<=`, `>`, `>=` on numbers, strings and booleans; `!`, `&&`
  and `||` on every primitive, `??`, `?:`, optional chains, `!`, `as` and `satisfies`; template strings and `+` with
  strings, numbers and booleans; `++`, `--`, `=` and `+=` as values; `in`, `instanceof`, `delete`; calls of top-level
  functions with default and rest parameters and spread arguments, of function variables and of closures written in
  place; array methods `size`, `isEmpty`, `push`, `pop`, `shift`, `indexOf`, `includes`, `find`, `findIndex`, `some`,
  `every`, `map`, `filter`, `reduce` with and without an initial value, `forEach` and `join`; string methods `size`,
  `upper`, `lower`, `rep`, `split`; map and set methods `get`, `set`, `has`, `add`, `delete`, `size`, `isEmpty`;
  element reads guarded by a bounds check.
- Statements: `let` and `const`, shadowing in inner blocks, compound assignment and logical assignment (`??=`, `||=`,
  `&&=`), `if`, counting `for` loops up and down, `for...of` over arrays, strings, maps and sets, `forEach`, bounded
  `while` and `do...while`, `break`, `continue`, `switch` with fallthrough and `default`, `try` with `catch` and
  `finally` and `return`, `break` or `continue` inside them, `throw` of strings and integers, array and object
  destructuring with holes, defaults, rest and nesting, destructuring assignment, swaps, blocks, and functions declared
  inside a block.
- Mutation of arrays, maps, sets, object fields and record fields through variables, including inside closures and
  callbacks.
- Top-level functions, recursion, hoisting (functions printed after the code that calls them), and classes with fields,
  field initializers, constructors, methods, `this`, static fields and methods, subclasses with `super(...)`,
  `super.m(...)` and overrides.

Programs terminate and are deterministic: loops have constant bounds or a guard counter, the only recursion counts
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
| `%` of a negative number or by a negative number | known.md, "% is floored". |
| `**` by any exponent but 0, 1, 2 and -1 | known.md, "** is C's pow". |
| A number other than an integer turned into a string | known.md, "A number becomes a string through tostring". |
| Strings that are not ASCII | known.md, "A string's size counts bytes", and `split`, `upper` and `lower` work on bytes too. |
| Truthiness of numbers and strings without the lint suppressed | CRU032 rejects it, because `0`, `NaN` and `""` are truthy in Luau. The generator writes it with `// oxlint-disable-next-line roblox-ts/lua-truthiness`, so the lowered `&&`, `\|\|`, `!` and conditions are checked. |
| `null`, `typeof`, `==` and `!=` | The compiler rejects them, as roblox-ts does. |
| Arrays that hold `undefined` | Luau's `#` may return any border of a table with holes. roblox-ts documents it. |
| `sort` | Luau's `table.sort` is not stable and takes a boolean comparator, so the two are different functions. |
| Getters, setters, private names, labels, generators, `async`, `await`, regular expressions, bigints, tagged templates, class expressions, object rest | The compiler rejects them. |
| Indexing an array out of bounds | JavaScript gives `undefined`, Luau gives `nil` and the type says the element exists; the generator guards each read. |
| Iteration order of maps and sets | A Luau table has no insertion order, which roblox-ts documents. Traces sort map and set entries, and loops sum order-free terms. |
| Negative operands of bitwise operators, `~`, and shifts past 8 | known.md, "Bitwise results are unsigned". |
| Errors thrown by the engine, such as calling `undefined` | Both sides fail, with messages that differ by design. Node reports one as a fault of the generator. |
| Generic functions, interfaces, enums, namespaces, `import` and `export` | Out of scope for one-file programs of this test; each needs its own runner support. |
