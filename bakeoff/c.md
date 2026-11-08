| Score | Result |
| --- | --- |
| Golden | **PASS: all 197 files identical**, with no reorder. |
| Arch | **FAIL:** rule 1 **33 → 28**, no new rule 1 entries; **5** new rule 4 locations. |
| FD full build | Trunk median **11.084809 s**, C median **12.516226 s**; C/trunk **1.129133**. |
| Trait and contexts | **2** Extension methods; **25** extraction/query/lowering methods; **4** dispatch/fact entry kinds plus **1** helper descriptor. |
| New compiler surface | **44** directly named public items (**32** types/traits/aliases), plus **30** indirectly reachable compiler types; **90** tree methods become reachable. |
| Move cost | **40 files, +2,489 / −1,253** including bakeoff artifacts. Table construction is **63** lines; finish-program is **6** lines. |
| Required verification | Formatting, Clippy, workspace tests and `pnpm check` **pass**; architecture **fails**. |

Prototype C is on `proto/extension-c`, based on `caa8477ed991d625456e5e8ab09b6e6a2aaa0a18`. It adds `crucible-transform`, which depends on the compiler; the CLI composes it through `compile_with`. No PR or ABI crate is created. Runtime emit sources are unchanged. The existing UI/class-style paths stay in place for S1; the new seam has neither hook.

The compiler owns traversal and dispatch. Callee symbols resolve once alongside the remaining legacy macros, then use a symbol-to-hook index. Lowering uses recorded hook IDs. Class entries match base-class symbols; type entries match type-reference symbols; fact entries declare produced payload kinds and optional value lowering. Boot, registration, DI checks, type-ID formatting, attribute checks, and their payload/lowering code live in the transform. Shared legacy metadata, class-kind queries and diagnostics remain in the compiler for the remaining declarations and lints. `LowerCx::helper(module, name)` records imports on use, replacing `uses_crucible`; `uses_ts` still supplies `TS.import`. Opaque facts are stored in the compiler's per-site table, with their concrete types owned by the transform.

`Extension` has only `hooks` and `finish_program`. The latter expands queued boot globs after extraction; global scheduling and finalization are the part the table does not express. The erased-fact `Payload` protocol has another **2** methods, blanket-implemented. The concrete registrations are **4** call entries, **1** class entry, **1** type entry, **3** fact entries and **1** helper-module descriptor. There are no file-wide trait callbacks.

The public extraction context has **14** methods and **3** public fields, the metadata query service has **5** methods, and the lowering context has **6** methods: **25** total. Additionally, the exposed game/Rojo services have **8** methods, `Code` has **1**, and `Value` has **2**. These counts exclude the raw checker API accessible through the checker field. Extraction uses one shared context rather than separate call/class/type facades; lowering has a smaller wrapper.

The **44** new directly named items comprise **14** seam types/traits/aliases, **7** tree types, **11** shared/re-exported types, **10** free functions and **2** modules. Counts exclude methods, fields and enum variants. Public signatures and fields also make **30** more existing compiler types reachable through inferred values: **23** fact types, **2** macro types, **4** declaration types and `Rojo`. Thus **62** compiler types are directly or indirectly exposed. Publishing the tree also exposes **90** methods. The name inventory is in `bakeoff/c-surface.json`. This implementation spells **11** tsrs types: `tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile, Symbol, SymbolFlags}`, `tsrs_checker::{SignatureKind, Type, TypeFlags}`, and `tsrs_core::P`. Public signatures additionally expose `tsrs_checker::Checker` through metadata queries and `tsrs_compiler::CheckerHandle` through the checker field; the transform does not import those names. This is a substantial frontend dependency despite the small trait.

FD was copied into this worktree's `build/bakeoff/fd` using `git ls-files --cached --others --exclude-standard`, following the existing golden helper's approach. The original `D:\src\trio-forge\fragile-delivery` checkout was only read; its HEAD is `5514468f4d79e17bc2256b4da9fce17cbeb48e7a`. Dependencies were installed in the copy and Rogen generated both projects before timing. Each measured command was a complete `crucible build` of the job and warehouse compilation units, including emission and Rojo. Before every run, the copy's outputs, includes and place files were removed outside the timer. Both binaries use the stock release profile, including thin LTO. No Cargo job from this worktree ran during timing. The laptop is shared Windows hardware, Ryzen 9 5900HS, 16 logical processors; load was not controlled.

| Pair, trunk first | Trunk seconds | C seconds | C/trunk |
| --- | ---: | ---: | ---: |
| 1 | 9.565494 | 9.454470 | 0.988393 |
| 2 | 12.275606 | 12.516226 | 1.019601 |
| 3 | 8.781595 | 9.299221 | 1.058944 |
| 4 | 11.084809 | 12.637485 | 1.140072 |
| 5 | 13.523100 | 18.057785 | 1.335329 |

The ratio of medians is **1.129133**; the median paired ratio is **1.058944**. Full precision is in `bakeoff/c-times.json`, and `bakeoff/fd-time.py` reproduces the paired runs against two saved binaries and a prepared copy. The first pair also compared **1,212** emitted Luau modules, all identical. C is **12.9%** slower by the ratio of medians in this run. Paired ratios range from **0.988393** to **1.335329** under uncontrolled shared load, so these measurements do not establish the cause. C demonstrates no full-build speed benefit, and these timings do not isolate lowering. Both medians exceed the architecture's **3 s** budget. Call dispatch is indexed, but class/type extraction adds a traversal, and generic expression lowering now checks extension facts.

Move-cost detail: `git diff --stat origin/main` reports **40 files changed, 2,489 insertions(+), 1,253 deletions(-)**. Excluding `bakeoff/`, it is **36 files, +2,085 / −1,253**. The transform's policy, emitter and payload files contain **899**, **96**, and **49** lines respectively. Boot needs two side-entry call registrations, a literal-fact registration and the finish queue. DI needs two call registrations, class and type registrations, and two fact registrations. They share the **63**-line builder, a **20**-line state/context-adapter region, and **12** lines of compile/analyze composition wrappers. Five integration-test files move to the transform to avoid a reverse compiler dev-dependency; generic fixtures remain under the compiler. Their support module copies **224** lines. Test compilation now selects a uniquely named framework example, the diagnostics guide check scans both crates, and CI selects the moved suites. A regression test distinguishes the root attribute macro from the same-named emit function.

| PR 5 move | What this shape makes easy | Where it needs more machinery |
| --- | --- | --- |
| Declarations and schemas | Call-symbol entries and transform-owned typed payloads fit the existing schema emitters. | Declaration catalogs, reachability and macro-as-value checks need program state and non-call sites. |
| Typed lints | A finite symbol or base-type rule can register an entry. | Whole-file policies and predicates on inferred types cannot be reduced to a finite name table. A file pass or broad typed-site hook would enlarge the trait/table. |
| Rojo parsing | Boot already consumes mapping queries rather than parsing in its handler. | The prototype still exposes `Game`/Rojo from the compiler. PR 5 must move parsing and replace this service with an instance map. |
| Virtual modules | Resolved virtual-module symbols could use the same dispatch table. | Modules and declarations must exist before checking; the post-check hook builder is too late. This belongs in compiler inputs, not a lowering callback. |

The initial class matcher missed concrete mixins until it expanded intersection types and preserved instantiated bases; the CRU013 regression test caught this. A base-symbol table still needs correct type-graph traversal. Risks also include the string-keyed erased facts, first-match ownership of overlapping call/class entries, the extra fact/index storage and traversal, and the broad raw-checker dependency. The legacy metadata resolver and helper-path service still tie the extraction context to the remaining framework passes. The trait is smaller than the architecture sketch (five methods after S1's JSX-hook deletion), but callbacks, descriptors and public query services carry the complexity instead. The table makes matched lowering explicit; it does not make the whole transform declarative.

`cargo xtask arch --base origin/main` exits **1**. Rule 1 falls from **33** to **28**, with no new rule 1 violation or dependency/environment violation. There are exactly **5** new rule 4 locations, listed below. Consequently I cannot confirm that the full architecture check has nothing new. The violation list removes **10** eliminated entries and adds none; the checker is unchanged. Rule 4 requires ABI literals to live in `crucible-abi`, while this prototype deliberately has no ABI crate. Moving the implementations relocates four existing ABI names and introduces the transform's helper-module literal.

```text
4 crucible-transform:src/emit.rs "defineClass"
4 crucible-transform:src/lib.rs "@trioforge/crucible/emit"
4 crucible-transform:src/lib.rs "attribute"
4 crucible-transform:src/lib.rs "resolve"
4 crucible-transform:src/lib.rs "resolveAll"
```

Passed commands: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm check`, `pnpm build`, `cargo xtask spec check`, `cargo xtask golden check`, `cargo test -p crucible-transform -- --ignored`, `node --test crates/cli/npm/package.test.mjs` (**10** tests), and `pnpm run test:compiler` (**1,212** specs). `pnpm check` also includes **1,212** Lune specs. `cargo clean` completed from the repository root, removing **50,733 files / 11.3 GiB** from this worktree.

I would keep indexed tables as a dispatch mechanism, but would not select this prototype as the complete PR 4 seam. It has no demonstrated speed win, exposes too much extraction machinery, and needs another execution model for whole-file/type-driven policies. Its value is a concrete alternative tested through byte comparisons, with costs that can be compared with A and B.
