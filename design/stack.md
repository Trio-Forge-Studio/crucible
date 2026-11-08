# Crucible on Charm and Vide

Crucible builds on Charm 0.11 for state and Vide 0.6 for UI, and keeps what only it can do: object lifetimes, the compiler, the CLI, data and the network. The rule is one copy of each library in a place, and no Crucible code that a maintained library already does. This document records the stack, the evidence it rests on, what gets deleted, and the order of the pull requests that move Crucible onto it. It replaces `ui.md`.

## Evidence

The claims below cite Lune prototypes and source lines. The prototypes ran on 2026-11-07 in Lune 0.11.0 against `@rbxts/charm` 0.11.1 and `@rbxts/vide-charm` 0.4.1 from npm. They live in the planning folder `~/.hstack/docs/crucible-charm-vide/prototypes/`, not on a branch. Run each with `lune run <file>` from that folder. Every quoted output line below comes from such a run. The folder's `vc/` holds the unpacked `@rbxts/vide-charm` 0.4.1 package.

| Prototype | What it shows |
| --- | --- |
| `timing.luau` | When effects run: at creation, on every write, and once per `batch`. |
| `handler.luau` | Why Crucible batches the entries that cannot yield. |
| `errors.luau`, `blast.luau` | What a throwing effect does to the effects queued after it. |
| `ownership.luau` | One detached `effectScope` per object owns the effects that object creates. |
| `yield.luau` | Charm's active scope does not follow a thread across a yield. |
| `stall.luau` | A `batch` that yields holds every effect in the VM. |
| `types.luau` | A Charm atom and a computed are plain Luau functions. |
| `strict-probe.luau`, `cleanup-probe.luau` | What goes wrong when every effect and cleanup attaches to the object's scope, and where the owner is lost in strict mode. |
| `boot-probe.luau` | A batch does not defer a new effect's first run. |
| `context.luau` | The owner and lifetime rules below, modeled on Charm's own source. |
| `verify-r2-context.luau` | Raw Charm callbacks see the writer's context, not the registering object's. |

The prototypes in the first seven rows ran with Charm's `strict` off, because Lune has no `game` and Charm turns `strict` on only in Studio (`init.luau:70` to `:81`). The probes in the last four rows came from the S0 review and its fix. They load Charm's unmodified source, and all but `boot-probe.luau` set `Charm.flags.strict = true` before using it. They are small Luau models of the rules, not S2's code, and run no Crucible code. S2's Lune specs run with `Charm.flags.strict = true`.

Charm source lines refer to `@rbxts/charm` 0.11.1, file `src/init.luau`. Crucible source lines refer to `main` at `caa8477`.

## The stack

| Package | Role | How the game gets it |
| --- | --- | --- |
| `@rbxts/charm` | Values: atoms, computeds, batches | A required peer of `@trioforge/crucible` at `^0.11.1`. Never a hard dependency, never re-exported. |
| `@rbxts/vide` | UI rendering | A peer of the `@trioforge/crucible/ui` entry only. The root, server and client entries never import it. |
| `@rbxts/vide-charm` | Reads Charm values from Vide | The game installs it. Crucible does not import it. |
| `@trioforge/crucible` | Lifetimes, compiler ABI, data, network, the `ui` entry | The game installs it. |

The game lists Charm, Vide and `vide-charm` in its own `dependencies`. `vide-charm` 0.4.1 declares `@rbxts/charm` `^0.11.1` as a dependency and `@rbxts/vide` as a peer (`vc/package/package.json`). Crucible does not re-export Charm, because a re-export gives every value function two import paths.

## Values come from Charm

Charm's `atom`, `computed`, `untracked` and `batch` replace Crucible's `state`, `computed`, `untracked` and `Readable`. A `Readable<T>` becomes `() => T`, and a `State<T>` becomes Charm's `Atom<T>`.

- A Charm atom and a computed are plain functions. `types.luau` prints `atom type function computed type function`. Crucible's `State` is a callable table, which is why CRU037 exists (`docs/diagnostics.md:507`). With atoms as functions, CRU037 has nothing left to check, and it is deleted.
- An atom called with a function calls that function with the current value and stores the result (`init.luau:474`). Code that stores a function in an atom must wrap it. F1's codemod lists every such call in Fragile Delivery (FD).
- `replicated` values become getters `() => T`, and `publish` returns an `Atom<T>`. S2 makes that change. Games keep the same declarations.

## Lifetimes stay in Crucible

Crucible keeps `effect`, `watch`, `scope`, `onCleanup` and `own`. Each one tracks two separate things. The owner is the object, generation or enable interval that the running code belongs to, which lookup and disposal use. The lifetime is what a new effect, watch, scope or cleanup attaches to, and so what ends it.

Today Crucible finds the owner of running code from the thread: `current()` returns `ownerOf(coroutine.running())` (`packages/crucible/src/internal/lifecycle.ts:98`). The lifetime comes from the innermost frame first, and from `current()` only outside every frame (`packages/crucible/src/internal/reactive/ownership.ts:254` to `:261`). So an effect ends with its object, its parent effect run or its scope (`packages/crucible/src/reactive.ts:69` to `:70`).

Charm finds the lifetime from one module-level variable, `activeSub` (`init.luau:69`), which `setActiveSub` swaps (`init.luau:155`). A new effect links to whatever `activeSub` holds when it is created (`init.luau:614` to `:618`), so an effect created inside another effect is disposed when its parent reruns (`init.luau:600` to `:601`). `onCleanup` adds its function to the cleanups of `activeSub` (`init.luau:722` to `:724`), and an effect runs its cleanups before each rerun (`init.luau:212`).

That variable does not follow a thread. `yield.luau` sets object A's scope, yields, then resumes the thread while object B's scope is active, and creates an effect:

```text
active scope after A's hook yielded is A's: true
A disposed; A's late effect still runs (wrong owner): true runs 2
after B disposed, runs 2
```

The effect A's code created belongs to B. Disposing A leaves it running, and disposing B stops it. Game code that calls Charm's `effect` directly attaches to whatever scope is active when the call runs, not to the object whose code made the call. S3 adds CRU038, which rejects `effect`, `effectScope` and `onCleanup` imported from `@rbxts/charm` in game code.

### The lifetime depends on where the code runs

S2 picks the lifetime from where the code runs:

- **Inside a running Crucible effect or watch callback**, the lifetime is that effect's current run. Crucible leaves `activeSub` as the running Charm effect, which is Charm's default. A nested effect then dies on its parent's rerun, and `onCleanup` attaches to the current run, so it runs before the next one.
- **Outside any effect run**, the lifetime is the owner's detached `effectScope`. Each generation and each enable interval gets one, created on first use. Only then does Crucible set `activeSub` to the owner's scope, around the `charm.effect` call only.

`ownership.luau` shows the second case, with one detached scope set active around each entry:

```text
effect created in a hook ran 1
effect created in a later callback, total runs 2
after write, runs 4 (expect +2)
dispose ran onCleanup 1
after dispose, runs unchanged: 6
```

Effects created in a hook and in a later callback both belong to the object. Disposing its scope runs `onCleanup` once and stops both effects.

The owner's scope is wrong inside an effect run. `strict-probe.luau` and `cleanup-probe.luau` set it as `activeSub` around every effect and every cleanup, nested ones included:

```text
nested children responding after one parent rerun 2
cleanup via object scope after effect rerun 0
cleanup via object scope after object dispose 2
```

The nested effect becomes a sibling of its parent, so each parent rerun adds another child, and two children respond where one should. The cleanups wait for the object's disposal instead of running before each rerun. `scope`, `onCleanup` and `own` keep their meaning for games. S2 rebuilds them on these two lifetimes.

### Lookup reads a synchronous context first

Charm turns on `strict` when it runs in Studio (`init.luau:72`, `:81`). In strict mode it wraps user callbacks so that each runs on a new coroutine, and a yield raises an error (`init.luau:125` to `:127`, `:119`). Effect bodies (`init.luau:609`) and `batch` callbacks (`init.luau:770`) are wrapped this way. So is `untracked`'s callback whenever an effect or scope is active (`init.luau:743` to `:755`), and `subscribe` and `listen` call their callbacks through `untracked` (`init.luau:832`, `:807`). Inside any of these, `coroutine.running()` is not the entry's thread, so a lookup by thread finds no object there. `strict-probe.luau` restores the owner by thread in an effect body and loses it one level down:

```text
effect body owner restored true
untracked body retains owner false
```

So lookup reads a synchronous context first and the thread map second.

- Crucible keeps one module-level context: the owner and the lifetime. It sets the context, saved and restored in a `try`/`finally`, around every callback it hands to Charm: effect bodies, watch source and `onChange`, scope setup, and cleanups. Inside an effect run, the lifetime is the running Charm effect, read with `getActiveSub` when the run starts. A nested effect takes its lifetime from the context, not from `activeSub`, which `untracked` clears (`init.luau:745`).
- In strict mode Charm runs those callbacks on a new coroutine, but synchronously, and a yield raises an error (`init.luau:118` to `:119`). The context stays set for exactly as long as the callback runs, so it reaches through Charm's `untracked`, `subscribe` and `listen`, which add coroutines but never resume one later.
- Charm runs cleanups inside whichever write triggers the rerun, so the context there belongs to the writer. Each cleanup runs under the context recorded when it was registered.
- The thread map stays for entries that can yield, such as start hooks and network handlers. A yield ends any synchronous context, and `yield.luau` shows that nothing module-level survives one. The thread does.

`watch` is built from Crucible's own effect, not from Charm's `subscribe`. Charm's `subscribe` runs its callback through `untracked` inside an internal effect it creates (`init.luau:826` to `:832`), and `untracked` clears `activeSub` (`init.luau:745`), so wrapping only the callback cannot recover that effect's run. Crucible's `watch` instead reads the source in the body of a Crucible effect and calls `onChange` through `untracked` from that body. Both run in the synchronous context of the watch's owner, captured when the watch is created, and their lifetime is the watch's internal effect run, not the lifetime at creation. So a write from another object's entry still runs the source as the watch's owner, and the effects `onChange` creates die on the next change.

Game code that registers raw Charm `subscribe` or `listen` callbacks gets no Crucible ownership. Only Crucible's own entry points wrap the callbacks they hand to Charm, so a raw callback that a later write runs sees the writer's context.

`context.luau` models these rules, with A's effects and watches run by writes from B's entry:

```text
cleanup before parent rerun 1
nested children responding after one parent rerun 1
owner inside untracked true watch onChange true cleanup true
watch source retains A on B write true
watch nested children after two changes 1
```

The verifier's `verify-r2-context.luau` registers raw callbacks inside A's effect and writes from B:

```text
raw callback owners untracked true listen false subscribe false
```

It is a model in Luau, not S2's code. S2's Lune specs test each rule with `Charm.flags.strict = true`: a nested effect that dies on its parent's rerun, `onCleanup` before a rerun, owner lookup inside `untracked`, a watch's source and `onChange` and a cleanup, and an effect created in `onChange` that dies on the watch's next change.

## Effects run on the write

Charm runs an effect when it is created and again inside every write that changes what it read. Crucible's current effects run on a deferred flush instead, never inside the call that created them (`packages/crucible/src/reactive.ts:67`). That flush is deleted. `timing.luau` prints:

```text
after create, runs=1
after write, runs=2
two writes unbatched, runs=4
inside batch, runs=4
after batch, runs=5
diamond 4 6
diamond runs=2
read-after-write dead=true
writer sees effect error: true ...timing:30: effect failed
```

The last line's file path is shortened.

- An effect runs once at creation, and once per write outside `batch`.
- Inside `batch`, writes queue the effect, and it runs once when the outermost batch ends (`init.luau:768` to `:777`, `:293`).
- An effect that reads two computeds of one atom runs once per write and sees both updated (`diamond 4 6`).
- A value an effect writes is readable right after the write that triggered it (`dead=true`).
- An error thrown by an effect reaches the code that wrote the atom.

### Crucible batches every entry that cannot yield

An effect that throws inside an unbatched write stops the writer at that write. `handler.luau` runs the same handler twice. It sets `hp` twice, which makes an effect throw, then awards 10 gold:

```text
plain handler: handler finished=false, gold=0, effect runs after create=2, error surfaced=true
handler in batch: handler finished=true, gold=10, effect runs after create=1, error surfaced=true
```

Unbatched, the handler stops at the write and the gold is never awarded. Batched, the handler finishes, the effect runs once, and the error still surfaces when the batch ends.

So Crucible batches every entry that cannot yield. These are `Reconciler.enter`, which runs lifecycle hooks (`packages/crucible/src/internal/lifecycle.ts:345`), and boot.

Crucible opens these batches with Charm's `startBatch` and `endBatch` (`init.luau:285`, `:293`), which run no callback, instead of `batch(fn)`, whose callback strict mode moves to a new thread (`init.luau:770`). The pair brackets the `runner.call` boundary (`packages/crucible/src/internal/runner.ts:44` to `:64`): the batch opens before `call` resumes its worker and closes after the resume returns, whatever the result. `call` already detects a yield and cancels the worker, so a `Yielded` result or a cancellation closes the batch too. A batch is never left open inside a coroutine that can be suspended.

Charm's strict mode does not guard this. It catches yields only in the callbacks it wraps, and `startBatch` and `endBatch` wrap none. `strict-probe.luau` opens a manual batch in a coroutine with `strict` on, yields, then writes an unrelated atom:

```text
strict manual entry yield accepted true unrelated seen 0
seen after resume 1
```

The yield is accepted, and the unrelated effect waits for the batch's thread. Crucible's runner is the guard, in Studio as in Lune.

### Boot keeps its deferred first run

Today no effect runs before boot commits. An effect's first run waits for the next flush, never inside the call that created it (`packages/crucible/src/reactive.ts:67`), and boot holds that flush from before it constructs objects until it commits or rolls back: `holdBarrier` sets `held`, and a flush is never requested while `held` is set (`packages/crucible/src/internal/reactive/flush.ts:79` to `:92`, called at `packages/crucible/src/internal/boot.ts:228`, `:308`, `:313`).

A batch cannot replace that hold. Charm runs a new effect's body inside `effect` itself, batch or not (`init.luau:623` to `:636`). `boot-probe.luau` creates an effect inside `startBatch`:

```text
effect first run before boot batch closes 1
effect runs after boot batch closes 1
```

So while boot is open, Crucible's `effect` and `watch` create no Charm effect. They queue the owner and the body, and return a disposer. When boot commits, Crucible creates the queued effects in order inside one `startBatch` and `endBatch`. On rollback it discards the queue, and nothing runs.

Queued entries are owned resources:

- Each entry registers with its owner's generation or enable interval when it is queued.
- Ending that owner cancels the entry, whether by explicit disposal, destruction or the end of an interval. The returned disposer cancels it too, and after commit it stops the created effect.
- At commit, Crucible skips every cancelled entry and every entry whose lifetime has ended. That includes an object that an earlier entry's body destroys in the same commit.

Crucible must check each entry before it creates the effect. Charm does not refuse a new effect under a stopped scope: it links the effect to the active scope (`init.luau:617` to `:618`) and runs its body at once (`init.luau:623` to `:636`). Disposing the owner's scope therefore cannot cancel an entry that is not yet a Charm effect.

`context.luau` models the queue. A watch is queued like an effect, so its `onChange` waits for commit too:

```text
boot effect runs before commit 0 after commit 1
boot effect runs after rollback 0
queued effect disposer type function
watch onChange before boot commit 0
cancelled queued effect runs at commit 0
destroyed owner effect runs at commit 0
object destroyed by an earlier entry at commit runs 0
```

Outside boot, an effect runs at creation, as `timing.luau` shows. S2's Lune specs test an effect created during boot that runs only after commit and never after a rollback, a watch whose `onChange` waits for commit, and entries cancelled by their disposer, by their owner's end before commit, and by an earlier entry's body during commit.

### Crucible never batches an entry that can yield

Charm keeps one batch depth for the whole VM (`init.luau:66`). `stall.luau` opens a batch in one thread, yields inside it, and writes an unrelated atom from another thread:

```text
while another thread's batch is yielded, an unrelated write reached its effect: false seen 0
after that thread resumed and closed its batch, seen 1
```

A yielded batch holds every effect in the place until its thread resumes. A `batch(fn)` fails instead in Roblox Studio: Charm enables `strict` there (`init.luau:81`), and in strict mode a yield inside `batch` raises "Attempted to yield in an effect or scope" (`init.luau:119`). A manual `startBatch` stalls in Studio too, as `strict-probe.luau` shows above. Network handler threads and `runStart` can yield, so Crucible never batches them. A write in a yielding entry runs its effects at once, as in `timing.luau`.

### A throwing effect skips its siblings for that write

In Charm 0.11.1, when one effect throws during a flush, the effects queued after it are skipped for that write. The flush marks them without running them (`init.luau:265` to `:271`), then rethrows the error (`:277`). `errors.luau` makes effect A throw for every value above 0, and lets sibling B record the value:

```text
unbatched: writer ok=false, sibling effect saw 0, atom=1
unbatched: next write ok=false, sibling saw 0
batched: writer ok=false, sibling effect saw 0, atom=1
batched: next write ok=false, sibling saw 0
```

`blast.luau` makes A throw only for the value 1:

```text
write 1 ok: false
after failure, sibling saw 0
write 3 ok (A no longer throws): true sibling saw 3
unrelated new effect first run saw 0
unrelated atom write, its effect saw 5
```

The skip lasts for the failing write only. The next write that A survives reaches B again, and effects on unrelated atoms are unaffected. `batch` does not change it. The atom keeps its new value either way (`atom=1`).

A fix is proposed upstream as U1: run every queued effect, rethrow a single error unchanged, and combine two or more errors into one, as Charm already does for cleanups (`init.luau:197`). Crucible does not depend on that fix.

## One copy per place

A place must hold exactly one Charm and one Vide. Charm keeps its graph in module-level variables: the effect queue, the batch depth and `activeSub` (`init.luau:61` to `:69`). Two copies are two graphs. An atom from one copy is not tracked by an effect from the other, and a `batch` in one does not hold the other's effects.

Packages find each other in two ways, and both reach the game's single copy.

- **Compiled TypeScript** imports a package through `TS.getModule` (`crates/compiler/roblox-ts/RuntimeLib.lua:17` to `:45`). It walks up from the calling script and returns the first `node_modules/@rbxts/<name>` it finds. The game's Rojo project maps the game's `node_modules/@rbxts` as one folder under `rbxts_include.node_modules`, and maps only the runtime's `out` folder under `@trioforge/crucible` (`examples/sample-game/default.project.json:14` to `:25`). Crucible's runtime and the game's code both walk up to that one `@rbxts/charm`. Mapping the runtime's whole package folder would put the runtime's own `node_modules` first in the walk (`AGENTS.md`, the `examples/sample-game` entry).
- **`vide-charm`** does not use `TS.getModule`. Its `Charm.luau` is `return require(script.Parent.Parent.charm.src)`, and its `Vide.luau` is `return require(script.Parent.Parent.vide.src)` (`vc/package/Charm.luau:1`, `vc/package/Vide.luau:1`). It always loads the sibling folders next to it, which are the game's own Charm and Vide.

That second path ignores any nested copy a package manager installs for `vide-charm`. If the game's Charm falls outside `^0.11.1`, npm can nest a second Charm under `vide-charm`, whose types the editor reads while the place still runs the game's copy. S5 makes `crucible build` fail on a second copy, or on a root copy outside a range that an installed package declares.

Charm is a peer of the runtime, not a hard dependency. The place loads packages only from the game's mapped `node_modules/@rbxts` (`examples/sample-game/default.project.json:14` to `:16`). Under pnpm, a hard dependency of the runtime puts no Charm there, so the game must list Charm anyway. When the two versions differ, the hard dependency adds a second copy of Charm's types. A peer makes the game list Charm once, and the package manager checks the range.

## What gets deleted

- **Crucible UI.** The `@trioforge/crucible/ui` renderer built on Crucible's reactive engine is deleted: `mount`, `Show`, `For`, `bind`, `append` and its JSX types. With it go the `react-jsx` lowering, the class compiler, `className`, `theme.toml` and CRU037. `jsx: "react-jsx"` becomes a compile error that names `jsx: "react"` with `jsxFactory: "Vide.jsx"` as the fix. CRU028 to CRU031 stay retired, and no later rule reuses their numbers. S1 does this.
- **The reactive engine.** `state`, `Readable`, `State`, the reactive graph, the deferred flush, the reactive context, the `@trioforge/crucible/vide` bridge and its optional Vide peer, and the flush helper in `@trioforge/crucible/testing`. S2 does this.
- **Anvil as a package.** Anvil's headless primitives move onto Vide in `@trioforge/crucible/ui`. The Anvil repository is archived and `@trioforge/anvil` is deprecated after the move. S4 does this.
- **`design/ui.md`.** This document replaces it.

## The pull requests

The work ships as these pull requests, in this order. S0 records the design. S1 deletes Crucible UI. S2 moves reactivity onto Charm. S3 adds a lint. S4 folds Anvil in. S5 adds a build check. S6 is a gated Charm Sync trial. S7 is the release. F1 ports FD. U1 goes to Charm upstream.

| ID | Pull request | Starts after |
| --- | --- | --- |
| S0 | Record the Charm and Vide stack in the design | Architecture PR 2 (#105) |
| S1 | Delete Crucible UI, `className` and `theme.toml` | Architecture PR 3 (#106). It merges before architecture PR 4 opens. |
| S2 | Run Crucible's reactivity on Charm | S1. It keeps every `emit` function unchanged. |
| S3 | Reject Charm lifetime calls in game code (CRU038) | S2. While architecture PR 5 is open, CRU038 goes into its typed-lint commit instead. |
| S4 | Fold Anvil into the `ui` entry on Vide | S2 |
| S5 | Fail the build on a second Charm or Vide copy | S2 and architecture PR 6 |
| S6 | Trial replication through Charm Sync behind a gate | S2 |
| S7 | Release the runtime, compiler types and CLI as 0.5.0 | S1, S2, S3, S4 and S5. S6 merged or closed. |
| F1 | Move FD onto Crucible 0.5.0 | S7 published |
| U1 | Make Charm run every effect when one throws | Nothing. It goes to `littensy/charm`. |

S4 and F1 change what players see, so each waits for a Studio review before merge. `roadmap.md` places these pull requests between architecture milestones A and B as milestone "Stack".

## What stays in Crucible

- **Lifecycle.** Subsystems, behaviours, generations and enable intervals stay Crucible's. Crucible finds the owner from its synchronous context, and by thread in entries that can yield (`yield.luau`, `packages/crucible/src/internal/lifecycle.ts:98`). Charm has no owner that survives a yield.
- **The compiler.** Registration, `inject`, `boot`, the declaration macros and the CRU rules depend on TypeScript types the compiler reads. The transform emits calls into `@trioforge/crucible/emit`, and S2 keeps every one of them unchanged, so the architecture lane's PR 4 and PR 5 work against the same ABI.
- **Data.** Profile stores keep Crucible's session protocol (`packages/crucible/src/internal/data/session.ts:19`). Moving saved profiles to another library needs its own migration plan.
- **The network.** `toServer`, `toClient`, `askServer` and `receive` are compiler macros (`crates/compiler/src/macros.rs:162` to `:165`), and the transform builds each message's schema from its TypeScript type (`crates/compiler/src/lower/crucible.rs:102`). No measurement shows that another network library is faster.
- **Replication.** `replicated` and `publish` stay Crucible's. S6 replaces their internals with `@rbxts/charm-sync` only if Charm Sync measures within 10% of Crucible's protocol on bytes per second and server Heartbeat time, and never exceeds the frame byte budget.

## Not yet proven

- Charm's cost per write and per value against Crucible's engine. S2's benchmark measures it.
- Charm Sync's bandwidth and Heartbeat cost. S6's gate measures it.
- Behavior in Studio. Lune proves lifecycle and resolution, not layout or input. S4 and F1 end with a Studio review.
- That no hook behind `Reconciler.enter` yields. If one does, the runner cancels it and its batch closes, so the hook fails instead of stalling the place. S2 tests a yielding entry that closes its batch, and a yielding network handler next to another object's writes.
- The ownership rules on Crucible's own code. `context.luau` models them. S2's Lune specs with `Charm.flags.strict = true` prove them.
