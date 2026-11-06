# @trioforge/compiler-types

The compiler types for Crucible's TypeScript 7 compiler.

This package is a copy of [`@rbxts/compiler-types`](https://github.com/roblox-ts/compiler-types) 3.0.0-types.0 by roblox-ts, under the MIT license in `LICENSE`. It changes one thing. `Iterable`, `IterableIterator`, `AsyncIterable`, and `AsyncIterableIterator` take `<T, TReturn = any, TNext = any>`. TypeScript 7 reports `TS2802` on every `for..of` over a Roblox iterable without those parameters, and TypeScript 5 accepts both forms.

Games built with `crucible build` use this package, and the release workflow publishes it with the CLI at the CLI version. The runtime and sample use TypeScript 7 configs. `test/compiler-types.test.mjs` checks the runtime and the sample game with tsrs.

`@rbxts/types` references `@rbxts/compiler-types` with a triple-slash directive, so a program that resolves that package loads both copies. TypeScript merges each pair of interface declarations, and the merged `Iterable` takes the three parameters. Lint skips `types` so the files stay comparable with upstream.
