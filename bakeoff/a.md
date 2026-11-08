# Bakeoff prototype A: tree view

The compiler publishes its tree view, the tsrs crates and a few fact and place types. A new `crucible-transform` crate depends on them and implements one `Extension` trait. Boot, registration and DI (`inject`, `attribute`, class registration and the class rules) moved behind the trait. The compiler crate no longer names them. Branch `proto/extension-a` from `caa8477`.

## Scores

| Score | Result |
| --- | --- |
| Golden | `cargo xtask golden check`: 197 files identical, at the first green build and again at head. Fragile Delivery's emitted `places/*/out` also hashes the same at trunk and head on all 10 benchmark runs (`a71500e909beeb54`). |
| Arch rule 1 | 33 before, 27 after. No new rule 1 line. |
| Arch rule 4 | 23 before, 23 after. Five ABI-name lines leave the compiler and five land in `crucible-transform`, so `cargo xtask arch --base origin/main` fails with 5 "new" lines (see Arch below). |
| FD build time | Full `crucible build` of both places, release binaries. Trunk median 7.006 s, head median 7.118 s, ratio 1.016. The machine was under load from another build, so this is within noise. |
| Trait size | 5 trait methods. 18 public methods on the context types. The compiler newly exports 4 modules (`ext`, `packages`, `tree`, `tsrs`) and 17 named items plus `compile_with`, and `tree` alone brings 7 types and 90 functions. The extension spells 14 tsrs types and 6 tsrs free functions. |
| Move cost | 18 source files, +2124 / −1384. About 90% of `crates/transform/src/extract.rs` is the old code with `self.checker` renamed to `self.cx.checker()`. Roughly 150 lines are duplicated, temporarily, because declarations still live in the compiler. |

Other checks at the pushed head: `cargo fmt --all --check` passes. `cargo clippy --workspace --all-targets -- -D warnings` passes. `cargo test --workspace` passes, and so does `cargo test -p crucible-transform -- --ignored` (Lune). `pnpm check` exits 0. `pnpm run test:compiler` passes 1212 Lune specs.

## The shape

```rust
pub trait Extension: Send {
    fn name(&self) -> &'static str;
    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: P<SourceFile>);
    fn finish_program(&mut self, cx: &mut ProgramCx<'_>);
    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr>;
    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, name: &Name) -> Vec<Stmt>;
}
```

- `compile_with(tsconfig, target, &mut [&mut dyn Extension])` runs the passes. `compile` is `compile_with` with no extensions. The CLI calls `crucible_transform::compile`. `Send` is required because the compiler runs on its own 512 MB stack thread. tsrs's `P<T>` is `Send + Sync`, so this costs nothing.
- Extraction calls `extract_file` after the compiler's own extraction of each file. It calls `finish_program` once after the last file.
- Lowering calls `lower_call` before the compiler reads a call's callee fact. It calls `lower_class_tail` after each class declaration.
- Extension facts live in the extension in a `HashMap<SiteId, Fact>`. `SiteId` is `{ file: FileId, span: Span }`. Extraction computes it from a tsrs node and lowering computes it from a tree `Node`, so one hash lookup answers each call.
- `LowerCx::helper(local, name)` returns `local.name` and records the import. `ProgramCx::helper_module(local, path, missing)` declares the module. This replaced `uses_crucible`, and `uses_ts` still works. The compiler's own declaration lowering, which has not moved yet, now calls `helper("__crucible", …)` as well, so the transform's `finish_program` declares that module for both.
- `boot` load globs are an expression inside the call's arguments, not the call itself. `LowerCx::substitute(span, expr)` makes the compiler lower one node to a given expression, and `lower_call` for `boot` fills it before it lowers the arguments. The compiler checks the map only when it is non-empty.
- Diagnostics go through `ExtractCx::report(node, Code, message)`, the compiler's own deduplicating `report`. The codes and the text are unchanged. `Code` moved from `extract/crucible.rs` to `ext.rs` and became public.

### Context methods (18)

- `ExtractCx` (9): `checker() -> &mut CheckerHandle`, `program() -> &'static Program`, `file`, `site(P<Node>)`, `span(P<Node>)`, `count_site`, `report`, `packages() -> &mut Packages`, `game() -> Option<&Game>`.
- `ProgramCx` (2): `game_module(path) -> Option<ModulePath>`, `helper_module(local, path, missing)`.
- `LowerCx` (7): `file`, `site(Node)`, `expr(Node)`, `macro_arguments(Node)`, `helper(local, name)`, `substitute(span, Expr)`, `unsupported(Node, what)`.

### What the compiler newly exports

- `pub mod ext` (8 items): `Extension`, `Code` with `render`, `FileId`, `SiteId`, `ExtractCx`, `ProgramCx`, `LowerCx`, `skip_parens_and_assertions`.
- `pub mod packages` (3 items): `Package` (`dir`, `json`, `name()`), `Packages` (`package_of`, `module_path`, `entry_module`, `invalid_metadata`), and `entry_types`. This is the npm-package and tsconfig-root lookup that used to sit inside `extract/crucible.rs`. It is generic, and both the compiler and the transform share one cache through it.
- `project` items (4): `Game` (fields `rojo`, `package`, `runtime_lib`; `load`, `out_path`, `out_folder`, `module`), `Rojo` (`load`, `is_game`, `rbx_path`, `import`), `RbxPath`, `normalize`. Only `boot`'s load-glob check needs these.
- `Span` and `ModulePath` are public without `test-support` now.
- `pub mod tree`: the whole lowering view, which is 7 types (`Node`, `Nodes`, `File`, `Position`, `Kind` with 113 variants, `Op`, `UnaryOp`) and 90 functions. The transform's lowering uses `Node`, `Kind`, 7 variants and 8 accessors.
- `pub mod tsrs`: re-exports `tsrs_ast`, `tsrs_checker`, `tsrs_compiler` and `tsrs_core`. tsrs is pinned once, in the compiler's manifest, and the transform has no tsrs dependency of its own.
- `compile_with`, plus `analyze_with` under `test-support`.

### tsrs types an extension must spell

The transform's extraction names 14 tsrs types: `P`, `Node`, `SourceFile`, `Symbol`, `Kind`, `ModifierFlags`, `NodeFlags`, `SymbolFlags`, `Checker`, `Type`, `TypeFlags`, `ObjectFlags`, `SignatureKind` and `LiteralValue`. It also calls 6 free functions: `get_combined_modifier_flags`, `get_source_file_of_node`, `is_external_module`, `is_global_scope_augmentation`, `get_heritage_clause_element_name` and `get_extends_heritage_clause_elements`. It receives `CheckerHandle` and `Program` without naming them. It calls 18 checker methods and about 45 node, type and symbol accessors, and it matches 31 syntax kinds. Lowering spells no tsrs type. For this move, the review estimate in `design/architecture.md` (27 checker methods, 35 node accessors and 68 kinds for all of framework extraction) holds: a facade would re-expose almost everything.

## Arch

- Rule 1 drops by 6: `macros.rs` loses `"attribute"`, `"inject"` and `"boot"`, `lower/crucible.rs` loses `"attribute"`, and `lower/stmt.rs` loses `"__crucible"` and the emit-missing message. Nothing is new. The remaining 27 belong to declarations, typed lints, UI and the CRU004 allowlist, and are PR 5's later steps.
- Rule 4 moves rather than shrinks. `"attribute"`, `"defineClass"`, `"resolve"` and `"resolveAll"` leave `crucible-compiler`, and they reappear in `crucible-transform:src/lower.rs`, plus `"attribute"` in `crucible-transform:src/extract.rs` as the macro's export name. The ratchet keys lines by crate and file, so `--base origin/main` reports the five as new even though the count is the same. Any shape that moves code into a transform crate hits this. PR 4 must land `crucible-abi` with the seam, or accept a one-time relocation in `arch.txt`.
- The test move did not add a rule 1 dependency, because the compiler has no dev-dependency on the transform. Rule 1 counts dev-dependencies (`workspace_dependencies` takes every dependency kind).

## FD build time

The FD checkout was never written to. A copy of FD `05a91cba` (git's tracked and untracked-but-not-ignored files, the way `xtask golden --fd` copies) is at `~/.cache/crucible-tmp/fd`. It ran `pnpm install --frozen-lockfile`, and a `@trioforge/crucible-cli` shim points it at a local binary. One `node scripts/place.mjs compile` generated the Rogen projects. Each timed run deletes `places/*/out` and both `.rbxl` files, then runs `<binary> build`, which compiles and Rojo-builds warehouse and job. The binaries are `cargo build --release -p crucible` at trunk `caa8477` and at this head. The machine is the 8 vCPU VPS. Another prototype was building at the same time, so the 1-minute load average was between 3.5 and 5.6 throughout. The script is `~/.cache/crucible-tmp/fd-bench.sh`.

| Run | Trunk (s) | Head (s) | Load at start (trunk / head) |
| --- | --- | --- | --- |
| 1 | 7.409 | 7.401 | 5.59 / 5.62 |
| 2 | 6.831 | 6.818 | 4.98 / 4.66 |
| 3 | 6.843 | 7.513 | 4.53 / 4.36 |
| 4 | 7.164 | 7.118 | 4.17 / 3.68 |
| 5 | 7.006 | 6.830 | 3.46 / 3.88 |
| Median | 7.006 | 7.118 | |

The ratio is 1.016. Head is faster in four of five pairs, and the median gap comes from run 3 alone. This shape adds three pieces of per-build work: a second full walk of each file's syntax tree, a second macro lookup per call, and a second scan of the runtime package's exports. None of them shows above the noise on FD. A quiet machine and more runs are needed to bound it below 1%.

## Move cost

`git diff -M --stat origin/main...HEAD` covers 157 files, +2239 / −1482. Most of the file count is the test folder rename. Source alone is 18 files, +2124 / −1384:

| File | Change |
| --- | --- |
| `compiler/src/ext.rs` | +166, new: trait, contexts, `Code`, ids |
| `compiler/src/packages.rs` | +199, new: moved out of `extract/crucible.rs` and made public |
| `compiler/src/extract/crucible.rs` | −1188 net: what remains serves declarations, `receive`, CRU004 and CRU023 |
| `compiler/src/lib.rs` | 70: modules, re-exports, `compile_with`, `analyze_with` |
| `compiler/src/lower/*` | about 200: helper registry, two hook calls, substitution, removed macro arms |
| `compiler/src/facts.rs`, `macros.rs`, `project.rs`, `extract/mod.rs` | about 110: variants removed, `Place.helpers`, visibility |
| `transform/src/extract.rs` | +1237, ported |
| `transform/src/lower.rs` | +131, ported |
| `transform/src/lib.rs` | +203: extension impl, fact types, `compile`, test-support `Analysis` |

Glue per feature:

- **Seam, shared by every feature.** About 300 lines: `ext.rs`, the two hook call sites (3 and 1 lines), extension calls in the extractor loop (10), the helper registry (30 in `lower/mod.rs` and `stmt.rs`), `FileId` threading, and visibility changes.
- **Boot.** `LowerCx::substitute` plus the map and its check in `Lower::expr` (about 12 lines). It also needs `ExtractCx::game`, `ProgramCx::game_module`, and `Game`, `Rojo`, `RbxPath` and `normalize` made public. The lowering is 8 lines. Boot needed the most new compiler surface for the least code, because its fact sits on a nested literal and its check reads the Rojo place.
- **Registration and DI.** `lower_class_tail`, the `Fact` enum, and a `Framework` that re-scans the runtime's exports for its three macros and the hook names (about 80 lines). It also duplicates `unalias`, `target`, `is_reference`, `is_global`, `bases`, `class_kind`, `kind_of`, `instance_type`, the library check, `side` and `macro_at` (about 70 lines). The duplication exists because `receive` and the declaration rules still need class kinds inside the compiler. It disappears when declarations move.
- **Tests.** The compiler cannot dev-depend on the transform without a rule 1 line, so every end-to-end test of Crucible games moved to `crates/transform/tests` as a rename. Only `tests/package-main` stays, for a compiler unit test. The adaptation is about 90 lines: a test-support `Analysis` that derefs to `Facts` and reads the transform's classes and facts, plus helper rewrites in `transform.rs`, `crucible.rs` and `fixtures.rs`. `--example compile` moved to the transform, and `test/compiler/*.mjs`, `package.json`, `vite.config.ts`, `ci.yml` and `AGENTS.md` paths followed. `test/docs.test.mjs` now reads CRU codes from both crates.

## Behaviour that changed outside golden's reach

- A spread as the last argument of `attribute`, `inject` or `boot` used to be an unanswered `MacroSpread` site. It is now a lowering error, "a spread argument to a Crucible macro is not supported". It still fails the compile, but with a different message.
- The compiler now computes generic call facts (`Callee::Function`, maybe `roblox_api`) for these three calls. Lowering ignores them because the extension claims the call first.
- The extension's walk also visits ambient code, where the compiler's visit skipped it. The facts it records there are never read.
- Without the transform, a program that uses declaration macros fails with "no extension declares the helper module __crucible". Before, it got the emit-missing message. This is transitional until declarations move.
- The transform's rules now report after the compiler's walk of the same file, not interleaved with it. `report` keeps the first message per path, position and code. No moved rule shares a node and a code with an unmoved one, and the tests found no difference.

## PR 5 under this shape

- **Declarations and schemas.** This move gets easier. `network.rs`, `data.rs`, `declarations.rs` and the schema lowering would port the way `crucible.rs` did, mostly by renaming `self.checker`. `network::Program` is a whole-program catalog with cross-file checks, which is exactly what `finish_program` is for. Once they move, the compiler's remaining `Api` (exports, declaration macros, widths, side entries, class kinds) moves too, and the duplication above disappears. The cost is more surface: `network.rs` leans on compiler-private helpers in `extract/types.rs` (`skip_downwards`, `is_any`, `walk_types`, the falsy and receiver logic). Each one must become public or be copied.
- **Typed lints.** CRU032 to CRU034 stay in the compiler and CRU035 to CRU037 move. Today they run per node inside the compiler's `visit`, with shared suppression-comment logic. The transform would walk again (cheap) and needs the suppression logic exposed, for example as `ExtractCx::report_lint`. The shape absorbs this with one more context method.
- **Rojo parsing.** This move gets harder. Boot made `Game` and `Rojo` public compiler API, and the transform calls `rbx_path`, `out_path`, `out_folder` and `package`. When Rojo moves to `crucible-project` and the compiler takes an instance map, the transform must reach the same answers through whatever map type the compiler holds. `ExtractCx::game` should return the instance map from the start, so the transform never learns what Rojo is.
- **Virtual modules.** These are compiler-only. Package lookups on a virtual path return `None`, so the transform treats such a module as not Crucible's. No trait change is needed.

## Risks

- **tsrs leaks across the seam.** The transform is as coupled to tsrs as the compiler is. A tsrs bump now edits two crates. Pinning in one place prevents version skew, not churn.
- **`pub mod tree` is a large surface.** It is internal, but every accessor becomes something the transform may call. It could shrink to the eight accessors lowering uses.
- **The helper registry is keyed by a string local.** The compiler's unmoved declaration lowering depends on the transform declaring `__crucible`. The registry should take a typed module id, perhaps from `crucible-abi`.
- **Arch ratchet.** `--base` fails on relocation until `crucible-abi` exists, as described above.
- **Duplicate work.** Two package scans, two macro lookups per call and two walks per file. The cost is unmeasurable on FD under this load. It stays bounded only while extensions are few.
- **Test location.** End-to-end tests of the compiler on real games now live in the transform crate. A compiler-only change runs them with `-p crucible-transform`, which is easy to forget.

## Verdict

The tree view is the cheapest shape to move code into. Golden stayed byte-identical on the first build, FD output did not change, the build cost is in the noise, and the 1237-line port was mechanical. That should hold for declarations, the largest remaining move. The price is that the seam is a crate boundary, not an abstraction. The transform names 14 tsrs types, 18 checker methods and dozens of node accessors, and the compiler's tree, package and place types become public. Boot showed where the trait is thin: a fact on a nested literal needed `substitute`, and a Rojo-dependent check pulled `Game` into the public API. If the bakeoff weighs move cost and output safety highest, pick this shape and narrow it. Keep `tsrs` behind the one re-export, make `tree` public only as far as lowering uses it, hand out an instance map instead of `Game`, and land `crucible-abi` in PR 4 so the rule 4 lines move once. If the score weighs a small, stable trait surface highest, this shape loses on purpose, because it exposes what a facade would hide.
