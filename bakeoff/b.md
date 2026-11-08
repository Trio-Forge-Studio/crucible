# Bakeoff B: narrow query facade

Prototype B of the PR 4 bakeoff. The transform never names a tsrs type. `ExtractCx` and `LowerCx` hand it compiler-owned handles (`Node`, `SourceFile`, `Symbol`, `Type`, `Signature`) and only the queries that boot, registration and DI use. Boot, class registration, `inject` and `attribute` now live in a new crate, `crates/transform` (`crucible-transform`). The compiler no longer names any of them. Branch `proto/extension-b`, based on `caa8477`.

## Scores

| Score | Result |
|---|---|
| Golden | `cargo xtask golden check`: **197 files identical**. No reorder. |
| FD output (extra) | Both FD places built by the trunk and head release binaries: **1,221 output files identical** (`places/*/out`, `places/*/include`). |
| Arch rule 1 | **33 → 27**. 6 lines removed, 0 new. |
| Arch rule 4 | 5 → 5. The same 5 ABI literals (`attribute`, `defineClass`, `resolve`, `resolveAll`, and the `attribute` macro name) moved from `crucible-compiler` to `crucible-transform`. `arch --base origin/main` fails on these 5 lines, because their location is new. No shape can avoid this until PR 4's ABI crate exists. |
| FD build time, batch 1 (load avg ≈ 10) | trunk median **8.169 s**, head median **7.913 s**, ratio **0.969** |
| FD build time, batch 2 (load avg ≈ 3.5) | trunk median **7.217 s**, head median **7.255 s**, ratio **1.005** |
| Trait | **5 methods**: `name`, `begin_program`, `extract_file`, `lower_call`, `lower_class_tail` |
| Context public methods | `ExtractCx` **31** (18 checker, 11 program, package and place, `report`, `declare_helper`). `LowerCx` **4**. |
| Handle public methods | `Node` 26 (+1 test-only), `SourceFile` 4, `Symbol` 4, `Type` 9, `Signature` 1. Flag types: 3 × 2 methods, 17 constants. `SyntaxKind`: 26 variants + `Other`. |
| Total public methods | **90** (5 trait + 35 context + 44 handle + 6 flag) |
| New public items | **15** in `crucible_compiler::extension`: `Extension`, `ExtractCx`, `LowerCx`, `Code`, `HelperModule`, `SyntaxKind`, `TypeFlags`, `SymbolFlags`, `Modifiers`, `LiteralValue`, `Node`, `SourceFile`, `Symbol`, `Type`, `Signature`. `compile` and `analyze` take the extensions. Under `test-support`, `Place::crucible_emit` became `Place::helpers: Vec<Helper>`. |
| tsrs types an extension spells | **None**. `crates/transform` contains no `tsrs` token. It still depends on tsrs transitively through the compiler. |
| Move cost | 78 files, +2,689 / −1,249, renames included. Compiler `src`: +836 / −1,164. Transform `src`: +1,468 new. Tests and scripts: +351 / −82, mostly moved files. |
| Facade glue | 688 lines in the compiler (`extension.rs` 409, `extract/extension.rs` 195, `lower/extension.rs` 84), plus about 60 lines of hook wiring in `extract/mod.rs`, `lower/{call,class,expr,stmt,mod}.rs` and `lib.rs`. |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass: 510 passed, 0 failed, 8 ignored |
| Ignored Lune tests (`-p crucible-compiler -p crucible-transform -- --ignored`) | pass: 4 passed |
| `pnpm check` | pass, exit 0 (1,212 Lune specs) |
| Not run | `cargo xtask spec check`, `node --test crates/cli/npm/package.test.mjs`, `pnpm run test:compiler` |

### How the FD time was measured

Both binaries are release builds of `crucible`: trunk at `caa8477` and head at `6e640bf`. The second commit, `b0409b4`, changes only tests, test-only accessors, and how the compiler's declaration lowering finds the `__crucible` helper. Golden was re-checked after it, and the outputs were still identical.

FD was copied from its git-tracked files into `~/.cache/crucible-tmp/fd-b`, as `xtask golden --fd` does, and set up with `pnpm install --frozen-lockfile` and FD's own `scripts/place.mjs compile`. The FD checkout was never written to.

Each run deletes `places/*/out` and both `build/*.rbxl`, then times `crucible build` of both places. Within each pair, trunk runs first.

| Run | Batch 1 trunk | Batch 1 head | Batch 2 trunk | Batch 2 head |
|---|---|---|---|---|
| 1 | 7.203 | 7.708 | 8.251 | 7.449 |
| 2 | 8.959 | 8.567 | 6.741 | 6.649 |
| 3 | 7.592 | 7.913 | 6.697 | 7.089 |
| 4 | 8.169 | 7.835 | 10.010 | 8.110 |
| 5 | 8.448 | 8.967 | 7.217 | 7.255 |

The VPS has 8 cores and was shared with another prototype's builds and FD bench. The spread inside one binary's runs, 2 to 3 s, dwarfs the gap between binaries. The honest reading is no measurable cost, not a speedup.

The absolute times are not comparable with the 3 s budget in `design/compiler.md`, which was measured on a laptop with no other load. Use the ratio. The extension adds a second full-tree walk per file and a second `macro_at` per call (symbol lookup, then resolved signature) on top of the compiler's own. The checker caches both, and neither shows up in these numbers.

## What changed

### The trait

```rust
pub trait Extension: Send {
    fn name(&self) -> &'static str;
    fn begin_program(&mut self, cx: &mut ExtractCx<'_>);
    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: SourceFile);
    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr>;
    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, local: &Name) -> Vec<Stmt>;
}
```

- **`finish_program` became `begin_program`.** Neither moved feature works after the last file. Both need the runtime package's export and macro symbol table before the first file, and the emit helper module declared before lowering. The network catalog, which PR 5 moves, will need `finish_program` back.
- **`lower_class_tail` takes the class's local name.** It returns statements instead of pushing them, so the compiler decides where they go.
- **Extension facts live in the extension, keyed by `Node`.** A `Node` is the tsrs pointer, so the node handed to `lower_call` is the same key `extract_file` stored. No span lookup is needed. The compiler's `Fact` lost `Crucible::Class`, `Crucible::Inject` and `Crucible::Load`. `Class`, `Dependency`, `Attribute` and `Check` moved to the transform.
- **`compile` takes `Vec<Box<dyn Extension>>`.** `crucible_transform::compile` registers Crucible's. The CLI's `__compile` child and the `compile` example call it.

### The helper registry

`LowerCx::helper(&HelperModule, name)` returns `<local>.<name>` and records the import. `HelperModule` is `{ package, entry, local, missing }`. The extension declares it in `begin_program`. The compiler resolves it once per program into `Place::helpers`. At the end of each file, the compiler imports every helper the file used, in order of first use. If the place cannot reach a used helper, the file gets the helper's `missing` error. The text of that error has not changed.

`uses_crucible` is gone. The compiler's remaining declaration macros look up the same helper by its local name, `__crucible`, through `declared_helper`. The transform declares it, so the compiler no longer spells `@trioforge/crucible` or its error text in `lower`. That removed 2 rule 1 lines that a compiler-side declaration would have added. `uses_ts` is unchanged. `uses_ui` is untouched, because S1 deletes it.

### Lowering `boot` needed one generic mechanism

`boot` replaces each load-glob string literal inside its options object with a `crucible-path:` pattern. The literal sits deep inside a call argument that the compiler lowers. Rather than add a string-literal hook, `LowerCx::arguments(call, replacements)` lowers a call's arguments with a list of `(Node, Expr)` substitutions. `Lower::expr` checks them only while the list is non-empty. That is the only glue specific to boot on the lowering side.

### Diagnostics

`ExtractCx::report` goes through the compiler's own deduplicated `report`, with the existing `Code` (`Rule(n)` and `Setup`). Codes and text are unchanged.

One ordering difference exists in principle. The compiler's remaining Crucible walk now runs before the transform's walk, where there used to be one interleaved walk. When two features report the same code at the same position with different text, the first one still wins. No test showed a difference. `test/docs.test.mjs` now reads CRU codes from `crates/transform/src` as well.

### Tests

A dev-dependency from `crucible-compiler` on `crucible-transform` would be a new rule 1 violation, because `cargo xtask arch` counts dev-dependencies. So the suites that need registration moved to `crates/transform/tests`, together with `transform-fixtures` and a copy of `support`: `transform.rs`, `crucible.rs` and `network.rs`.

The compiler keeps the generic suites, whose calls now pass no extensions. Its `fixtures.rs` lost the inject fact test, which moved to `crates/transform/tests/fixtures.rs`. Its macro-entry test now asserts that the compiler sees `boot` as a plain function. Facts-level class tests read the transform's facts through test-only `Crucible::classes()`, `injects()` and `calls()`.

`Facts::sites` no longer counts the transform's sites. It is a test-only statistic.

## Query inventory

Boot alone needs only `arguments`, `properties`, `elements`, `name`, `initializer`, `expression`, `text`, `kind`, `report`, `source_files` and the four place queries (`game_package_dir`, `out_folder`, `out_path`, `instance_path`). On the lowering side it needs `expr` and `arguments` with replacements. Registration and DI need everything else, including all 18 checker queries.

| Group | Queries |
|---|---|
| Checker (18) | `symbol_at`, `unalias`, `symbol_of_declaration`, `resolve_type_name`, `module_exports`, `type_at`, `type_from_type_node`, `declared_type`, `type_of_symbol`, `base_types`, `type_arguments`, `property_of_type`, `properties_of_type`, `is_assignable`, `construct_signatures`, `signature_of_declaration`, `return_type`, `resolved_signature_declaration` |
| Program, package, place (11) | `source_files`, `package_name`, `package_dir`, `package_metadata_valid`, `is_package_entry`, `module_id`, `is_library_declaration`, `game_package_dir`, `out_folder`, `out_path`, `instance_path` |
| Node (26) | `kind`, `parent`, `for_each_child`, `source_file`, `text`, `name`, `expression`, `initializer`, `type_node`, `type_name`, `arguments`, `type_arguments`, `has_type_parameters`, `members`, `properties`, `elements`, `modifiers`, `is_ambient_context`, `contains_this`, `has_question_token`, `is_rest`, `implemented_types`, `extends_types`, `is_extends_clause`, `heritage_name`, `is_global` |
| Type (9, no checker) | `flags`, `symbol`, `alias_symbol`, `types`, `is_reference`, `target`, `is_class_or_interface`, `type_parameters`, `literal` |
| Lowering (4) | `helper`, `expr`, `arguments`, `unsupported` |

### Testing the architecture prediction

`design/architecture.md` predicts that a facade would re-expose nearly the whole syntax tree, because framework extraction uses 27 checker methods, 35 node accessors and 68 syntax kinds.

Two features out of five took 18 checker queries, 26 node accessors and 26 kinds. Most of the checker surface is already paid for, because registration is the most type-heavy feature.

To extrapolate, I grepped what the framework code still in the compiler uses (`extract/network.rs`, `data.rs`, `typed_lint.rs`, the rest of `crucible.rs` and `declarations.rs`, 2,952 lines):

- **Checker:** 25 distinct tsrs checker methods. About 10 are not in the facade yet: `get_contextual_type`, `get_index_infos_of_type` (which needs an `IndexInfo` handle), `get_non_nullable_type`, `get_non_missing_type_of_symbol`, `get_base_constraint_of_type`, `is_array_type`, `is_array_like_type`, `resolve_external_module_name`, `type_to_string`, and call signatures.
- **Syntax kinds:** about 70 real kinds, about 50 of them new, including the binary operator tokens.
- **Node accessors:** 31 distinct `as_*` casts. That suggests 25 to 35 new node accessors.
- **Flags:** about 20 more flag constants.

The full framework would therefore need about 28 checker queries, 55 to 60 node accessors, about 75 kinds and about 37 flag constants: 150 to 170 public methods and an estimated 1,300 to 1,500 lines of facade.

**The prediction holds on size.** It does not hold on difficulty. Every query is a 1 to 5 line wrapper, and none needed design work.

## What the shape makes easy and hard for PR 5

- **Declarations and schemas: hard, mostly volume.** About 10 more checker queries, about 50 kinds and a new `IndexInfo` handle. The network catalog needs `finish_program` back. `type_to_string` must reproduce tsrs's text exactly, because messages embed it. The lowering needs one more query, an ordered lowering of selected arguments, for the kept arguments. `Schema`, `Field` and `MapKey` move out of the compiler cleanly. This move doubles the facade.
- **Typed lints: medium.** CRU032 to CRU034 move into the compiler with no facade cost. CRU035 to CRU037 run inside the compiler's main `visit`, which skips type nodes and ambient code. In this shape they need either a third full-tree walk in the transform or a new per-node trait hook. Either way, the order in which deduplicated diagnostics are reported needs care.
- **Rojo parsing: easy.** The facade already reduces the place to four queries: package dir, out folder, out path and instance path. Moving Rojo into `crucible-project` and handing the compiler an instance map changes only their bodies.
- **Virtual modules: easy, but a new trait method.** Sources must exist before the program is built, and `begin_program` runs after the check. That cost is the same in every shape.

## Risks

- **Every new framework rule changes the compiler.** A rule needing an unexposed checker call or syntax kind is a two-crate change. The facade becomes the bottleneck the seam was meant to remove. After PR 5, most new rules would reuse existing queries, but each new TypeScript construct still adds a `SyntaxKind`.
- **Duplicated state until PR 5 finishes.** The transform rebuilds its own export and macro symbol table and its own class-kind cache, because the compiler keeps its own for declarations, `receive` and typed lints. Each call is looked up twice. That cost is unmeasurable today and disappears once those features move too.
- **`Node` is a pointer.** It is valid only inside one program in one process, which matches the one-child-per-place build. A phase 7 output cache that persists facts would need span keys instead.
- **The `__crucible` helper is declared by the transform alone.** Without the transform, the compiler's own declaration macros fail with "no extension declares the `__crucible` helper module". That coupling ends when declarations move.
- **Rule 4 lines moved into `crucible-transform`.** The ratchet treats them as new until `crucible-abi` exists. No shape avoids this, but the bakeoff's arch score should not count it against B alone.
- **Diagnostic order.** See Diagnostics above. No test or golden showed a change, but the precedence between the compiler's walk and the transform's walk is now fixed by order, not by one shared walk.

## Verdict

The facade works. Golden and FD output are byte-identical, there is no measurable build-time cost, the transform names no tsrs type, and the trait stayed at 5 methods. It also produces a precise inventory of what the framework asks of the checker, which is useful for tsrs upgrades: tsrs is pinned and spelled in one crate.

It is expensive in the way `design/architecture.md` predicted. Two features already took 90 public methods and 688 lines of glue. The whole framework will take about 150 to 170 methods and about 75 syntax kinds, mostly wrappers over the syntax tree. The syntax half of the facade buys little: it re-exposes accessors one at a time that the compiler's existing `tree` view already wraps.

My recommendation is a hybrid. Keep this shape's checker facade, the handles and the helper registry, and give the transform the compiler's `tree` view for syntax instead of per-accessor `Node` methods. That would drop about two-thirds of the projected surface and keep tsrs out of the transform. As built, B is viable but probably loses to a shape that publishes the tree, unless the team weighs tsrs isolation above the speed of adding framework rules.
