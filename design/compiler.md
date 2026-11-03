# Crucible compiler design

The Crucible compiler is a proposed replacement for roblox-ts that compiles Roblox games written in TypeScript to Luau. It lives in this repository as crates in the `cli` workspace and runs inside the `crucible` binary, so `crucible build` compiles in process with no Node and no separate command. Its one overriding requirement is the speed and footprint of the compiler itself, so that many agents can build separate worktrees of one game on one machine. The quality of the emitted Luau is secondary, and matching roblox-ts output is not a goal. This document records the measurements taken on 2026-11-01, the design they lead to, and the phases that each end in a budget check. No product code exists yet.

## Goal and pass bar

The compiler supports the TypeScript features that Crucible and Fragile Delivery (FD) use, and nothing else. A feature the stack does not use is a compile error until someone needs it.

There is one mode. `crucible build` starts, compiles every file, writes the output, and exits. There is no watch mode and no resident process, because the team's workflow builds on demand.

A phase passes when the stack's own tests pass and the compiler meets these budgets on FD. The budgets are proposals derived from the spike numbers below.

| Budget | Target | Today on FD |
|---|---|---|
| `crucible build`, warehouse and job places, type check included | 3 s | 39.7 s and 36.9 s |
| The same build after editing one file | 3 s | 42.4 s |
| Peak memory of one build | 250 MB | 1,040 MB |

Correctness comes from the stack's tests, not from roblox-ts. These are Crucible's `pnpm check` with its Lune specs and transformer tests, FD's `lune run test`, and a Studio smoke run.

## Where the time goes today

A spike measured roblox-ts 3.0.0 on FD at `c7ef7706` on an 8 vCPU, 23 GiB VPS. FD's warehouse place compiles 599 source files, 739 with declarations. Numbers are medians of 5 runs.

- **Every edit is a full compile.** FD's pipeline deletes the tsbuildinfo whenever an input changes, because declaration-merged message guards go stale otherwise. A leaf edit and a signature edit both cost 42.4 s.
- **The Flamework transformer doubles the work.** With any transformer, roblox-ts reprints each transformed file and builds a second program, so it parses, binds, and checks everything twice. Without Flamework, a cold compile takes 23.0 s and 630 MB.
- **Checking dominates.** The cold CPU profile on the main thread is 19.5 s checking the second program, 8.2 s of Luau transform, 3.7 s of parse and bind, 1.3 s of checker setup, and 2.6 s of garbage collection.
- **Disk and Node startup are not the problem.** A cold compile with the page cache evicted took 40.2 s against 39.7 s warm. Node starts in 0.07 s and loads TypeScript in 0.41 s.

The same spike ran tsrs 0.20.1 (TypeScript 7.1 beta, git SHA `77d3cdd`) on FD under a TypeScript 7 config.

| Step on FD | Time | Peak RSS |
|---|---|---|
| Type packages plus an empty file | 0.10 s | 29 MB |
| Program build (parse and bind) | 0.16 s | 59 MB |
| Full check, 1 checker | 1.21 s | 94 MB |
| Full check, 4 checkers | 0.64 s | 134 MB |
| Fact queries for all 93,796 sites, no full check first | 1.85 s | 174 MB |

These figures use the system allocator. mimalloc with transparent huge pages on is 7 to 19 percent faster across these steps and uses 59 to 86 percent more peak memory. The type-package floor is bound by one file. `enums.d.ts` takes 27 ms of the 46 ms parse and bind, and one file cannot be split across threads.

## The design

Checking is the cost, and tsrs checks FD in about a second where roblox-ts takes about 20. The design spends that second once per build and avoids everything else.

### One process per build

`crucible build` runs these steps and exits.

1. Build one tsrs Program for all places in the worktree. FD has warehouse and job places, and sharing their parsed files in one process measured 206 MB against 260 MB for two processes.
2. Check every file. If there are type errors, print them and write nothing, as roblox-ts does.
3. Fill the fact table for every type-dependent site.
4. Lower each file to Luau.
5. Write only the output files whose bytes changed, so Rojo resyncs only those.

The operating system frees all memory when the process exits. The compiler needs no program lifetime management, no tsrs regions, and no invalidation rules. FD's merged message guards are always fresh because every build sees every file.

How much the fact queries cost after a full check is unmeasured. The 1.85 s above ran queries without checking first, so each query did its own checking. After step 2 most answers should come from the checker's caches (inferred). Phase 2 measures the full pipeline against the 3 s budget.

### One checker by default

The default is one checker on the system allocator. That measured 94 MB for a full FD check, against 249 MB for tsrs's default of four checkers with mimalloc and huge pages. Four checkers also cost 39 percent more CPU, which other agents' builds compete for. `crucible build --fast` selects four checkers and mimalloc for a single agent on an idle machine.

### Facts between the checker and the lowering

The lowering walks tsrs's syntax tree and reads type answers from a fact table. It never calls the checker. The table is keyed by node and holds the receiver kind, the callee kind (macro, method, or function), the falsy bits, `nullable`, the constant value, and Crucible's class metadata. The first spike answered every one of Crucible's 5,652 type-dependent sites this way. It also spot-checked about 35 sites against roblox-ts output, and all matched.

The earlier draft parsed with oxc and mapped spans onto tsrs nodes. The tsrs program already holds the syntax tree, so a second parse adds memory and a span-mapping step with no speed gain. oxc stays out of the compiler.

### No cache in version 1

Every build compiles every file. A per-file output cache would cut the edit build below the full build, but it brings back invalidation. A changed export must re-lower its importers, and a changed merged declaration must re-lower every reader. Phase 7 adds a cache only if the full build misses its budget or agents need a faster edit loop than 3 s.

### Types the compiler owns

`@trioforge/compiler-types` replaces `@rbxts/compiler-types`, with the three-parameter `Iterable` that TypeScript 7 requires. The compiler keeps reading `@rbxts/types` for the Roblox API. Splitting its `enums.d.ts` into chunks would save at most 60 ms per build, since removing that file entirely took the floor from 0.10 s and 29 MB to 0.04 s and 17 MB. Owning a fork of the Roblox API types costs more than that, so the split waits until a build budget needs it.

### Runtime interop

FD's runtime packages ship precompiled Luau that calls roblox-ts's `TS.import`. These include Flamework, janitor, log, message-templates, tether, serio, anvil, brigadier, and spline. The compiler keeps roblox-ts's `RuntimeLib` in `rbxts_include` for them. Those packages cost nothing at compile time beyond reading their declarations.

## Features in scope

These counts come from the spike's collector. FD has 117,409 type-dependent sites, and Crucible has 5,520.

- **Calls, indexing, operators, and loops.** Macro, method, and function callees. Array, Map, Set, and String receivers. LuaTuple returns. String `+` versus numeric `+`. Truthiness, `??`, numeric `for` loops, and `for..of` over arrays, maps, sets, and `pairs()`.
- **Classes and modules.** Classes, enums, `export =`, and `declare module` merging.
- **Other syntax.** `satisfies` and try with throw.
- **JSX for Vide.** 2,106 elements, of which only the 17 spread attributes need a fact.
- **Crucible's transform.** Registration, `attribute()`, `inject<T>()`, `boot()`, the emit import, and the CRU lint rules.

`async`, generators, and regex literals are compile errors, because neither codebase uses them. `const enum` compiles as a regular enum, which removes the checker lookup that import elision needed for 32 of 121 Crucible imports.

## TypeScript 7 configuration

FD's warehouse and job places reach zero errors under TypeScript 7 with these changes and `skipLibCheck`. Crucible's runtime and sample game need the first, third, and sixth rows.

| TypeScript 5 setting | TypeScript 7 replacement |
|---|---|
| `moduleResolution: "Node"` | `module: "preserve"` with `moduleResolution: "bundler"` |
| `baseUrl: "src"` | `paths` entries for each source root |
| `downlevelIteration` | removed |
| `types` defaulting to all packages | `types: ["*"]` or an explicit list |
| implicit `rootDir` | `rootDir: "src"` |
| `@rbxts/compiler-types` `Iterable<T>` | `@trioforge/compiler-types` |
| `plugins` transformers | built into the compiler |

Decorators, `jsx: "react"` with `jsxFactory: "Vide.jsx"`, and the Flamework and `@rbxts` typings worked unchanged.

## Phases

Each phase ends in a budget check and a test run that a reviewer can repeat.

0. **Delete `crucible dev`.** It is unused today and needs no compiler work, so it lands first as its own PR. Remove `cli/crates/crucible/src/dev.rs`, the command's registration, its section in `docs/cli.md`, and every pointer to it, such as the `npx crucible dev` step after `crucible create` and the template's `AGENTS.md`. The check is `pnpm check` and the CLI checks in `AGENTS.md` passing, with no remaining reference to `crucible dev` or `rbxtsc --watch`.
1. **Add the compiler types.** Add `packages/compiler-types`, the `@trioforge/compiler-types` source with the three-parameter `Iterable`, as a private workspace package. It is published through the release workflow later. Crucible's own configs stay on TypeScript 5 until phase 6, because `rbxtsc` builds them until then. The check is zero tsrs errors on Crucible's runtime and sample game under a TypeScript 7 test config that uses the package.
2. **Build the front end.** Add a compiler crate to the `cli` workspace that builds the tsrs program from a tsconfig, checks it, and fills the fact table, and writes nothing yet. Move the Rust pin to tsrs's version. The check is every site answered on Crucible's runtime and sample game in `cargo test`, and the whole run under 2.5 s and 250 MB on FD's TypeScript 7 copy.
3. **Lower Crucible's runtime.** Implement the lowering for the features Crucible's runtime uses. The check is Crucible's Lune specs passing on the compiler's output.
4. **Add the Crucible transform.** Port `transformer-src` into the compiler crates. The check is the sample game passing `pnpm check`, with the transformer tests ported to `cargo test` and passing.
5. **Cover FD's feature set.** Add JSX, `export =`, merged declarations, `satisfies`, and try with throw. The check is FD's tests passing and every budget in the pass bar met on FD.
6. **Move the stack.** Switch `crucible build` from `rbxtsc` to the in-process compiler. Remove `roblox-ts` and `typescript` from the template and the sample, delete `transformer-src`, and drop the physical `server`, `client`, and `emit` folders if `exports` resolution passes. The check is green CI on Crucible and FD.
7. **Add a cache, only if needed.** If the full build misses its budget, or agents need a faster edit loop, key each file's output by its content and the declarations it imports. A changed merged declaration invalidates every file. The check is an edit build under 500 ms with FD's tests still passing.

## Decisions

These are settled.

- **Home.** The compiler is crates in `cli/crates/` of this repository, linked into the `crucible` binary. It has no command line, binary, or npm package of its own, so it needs no product name. One PR can change the runtime's `@trioforge/crucible/emit` calls and the compiler's Crucible transform together, and a game can never run mismatched versions of the two.
- **Scope.** The compiler serves Trio Forge's stack only. There is no plugin API, and Crucible's transform is part of the compiler.
- **Pass bar.** Compiler speed and memory come first. Output quality comes second, and roblox-ts parity is not a goal.
- **Flamework.** The compiler never implements Flamework. FD moves to Crucible before phase 5. FD uses Flamework today on 201 decorated classes, with 351 injected constructor parameters, 65 macro calls, and 28 components with type-derived attribute guards.
- **No watch mode and no `crucible dev`.** Agents cannot connect Studio to `rojo serve`, because MCP gives no access to Studio plugins. Agents build the place and ask a person to open it in Studio, and people work the same way. Nobody runs the compile-on-save and sync loop, so the compiler has one mode, a full build per invocation, and `crucible dev` is deleted.

## Build and CI cost

Linking tsrs adds about 125 crates to a CLI whose lockfile lists 404 packages today. The spike's clean release build took 2:09 on 8 vCPU and produced a 16.5 MB stripped binary. Expect each platform's `@trioforge/crucible-cli` package to grow by about that much (inferred).

- **CI caching absorbs most of it.** Every Rust job in `ci.yml`, `cli-spec.yml`, and `release.yml` already uses `Swatinem/rust-cache`. Its key is the toolchain plus `Cargo.lock`, and tsrs is a git dependency pinned to one SHA in the lock. tsrs therefore compiles once per lockfile or toolchain change, and an ordinary PR rebuilds only Crucible's own crates.
- **Bumping tsrs or any dependency pays the full build once.** That is the price of a planned bump, and the budget suite runs on the same PR.
- **Incremental compilation stays off in CI.** rust-cache disables it, because incremental artifacts bloat the cache and help little on a fresh runner. Local builds keep Cargo's default incremental debug builds, so an agent editing a compiler crate recompiles only that crate.
- **The Rust pin follows tsrs.** tsrs requires 1.99.1, and the root `rust-toolchain.toml` pins 1.99.0. Phase 2 moves the pin to tsrs's version, and every tsrs bump checks it again.

## Risks

- **tsrs sits on the critical path.** The lowering reads tsrs's syntax tree. tsrs is four weeks old, does not publish its crates, and takes no outside pull requests. Pin a SHA, keep tsrs types inside two modules (the fact extractor and the tree view), and bump on purpose with the budget suite.
- **Fact queries after a full check are unmeasured.** If they cost close to the 1.85 s measured without a prior check, the build lands near 3.2 s and misses its budget. The fallbacks are four checkers, or querying facts while checking so each file is visited once. Phase 2 decides.
- **CI cache eviction.** GitHub keeps 10 GB of caches per repository by default. tsrs build artifacts for Ubuntu and Windows across several jobs may push older caches out, which turns an ordinary PR into a cold build (inferred). Watch cache hit rates after phase 2.
- **TypeScript 7.1 is in beta.** Retarget to 7.1 stable when tsrs does.
- **Licensing.** tsrs is Apache-2.0 with a NOTICE file that the binary ships. Code ported from roblox-ts keeps its MIT notice.

## Evidence

The spikes ran on `hezaerd-vps` and are local commits there, never pushed.

- `~/dev/rbxts-rs-spike/spike` at `f4b7692` holds the fact collector, the tsrs adapter, and the measurement harness. Results are in `spike/out`.
- `~/dev/rbxts-rs-spike/fd` is FD at `c7ef7706`, unmodified. `fd-ts7` is the TypeScript 7 copy, and `variants` holds the declaration experiments.
- The Crucible copy is `origin/main` at `86bede8`.

A 12-line lib file reproduces the `Iterable` break. Declare `Iterable<T>`, `IterableIterator<T>`, and a `Set<T>` with `[Symbol.iterator]()` under `noLib`, then write `for (const x of s) {}`. tsrs and `typescript@7.1.0-beta` report `TS2802`, and TypeScript 5.5.3 accepts the file. With `<T, TReturn = any, TNext = any>` on the iterator types, both TypeScript 7 compilers accept it.
