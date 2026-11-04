# UI renderer spike

## Round 2: trunk against the reactive-bind branch

Round 2 measures whether `perf/reactive-bind` closes the gap the first round found. It reruns this harness against two built runtimes. Trunk B is the spike's binding on a patched trunk flush, at `origin/main` 23bf3ea. Head B is the runtime's own binding node, built from the branch at fbfe74d, with one shared sink per property name, as a compiler would emit it.

Build trunk in a second worktree and the branch here, then run from the repository root:

```sh
lune run spike/ui/check <trunk out> <head out>
lune run spike/ui/bench <trunk out> <head out> 15        # every section; a fourth argument such as 24 picks some
pwsh spike/ui/pinned.ps1 -TrunkOut <trunk out> -HeadOut <head out> -Sections 2 -Runs 31   # High priority, one core
```

Each run of `bench` times every subject once, in an order that rotates by run. Section 2 also prints each run's head-to-Vide ratio, which cancels drift the subjects share. `profile.luau` splits head B's update into its write, flush, order check, numeric sort and property write. `variants.luau` compared the flush variants that were tried. Every run is in `results/`.

Other sessions kept 30% to 85% of the machine's 16 logical cores busy through every run, so absolute times swing by up to 2× between runs. The update rows were therefore rerun at High priority on one core, and the gate uses the paired ratios. Memory does not depend on CPU time.

| Row (1,000 elements) | Trunk B | Head B | Vide | Head ÷ Vide |
| --- | --- | --- | --- | --- |
| Mount, µs an element | 13.2 | 15.0 | 23.6 | 0.63 |
| Unmount, µs an element | 6.4 | 4.4 | 4.1 | 1.07 |
| Update, separate states, µs a prop | 4.13 | 3.20 | 2.51 | 1.24 paired (1.20 in the repeat) |
| Update, one shared state, µs a prop | 3.31 | 2.59 | 2.06 | 1.24 paired (1.51 in the repeat) |
| For reverse, ms | 11.6 | 10.2 | 6.9 | 1.48 |
| Memory, bytes an element, Instance included | 2,149 | 1,537 | 1,365 | 1.13 |
| Bridge create, first flush and destroy, µs a binding | 61 | 27 | | |
| Bridge update, µs a binding | 42 | 10 | | |
| Heap growth per state write, bytes | 128 | 0.25 | | |

Update rows are medians of 31 pinned runs (`pinned-updates-a.txt`), memory of 31 runs, and the rest of 15 pinned runs. Across all seven timed runs, head over Vide ranged from 1.08 to 1.78 on separate states and from 1.03 to 1.65 on one state. Memory ranged from 1.10 to 1.17 over three runs. `.lune/bench-vide` on the branch puts the bridge at 8.9 to 11.3 µs to create and 4.9 to 5.2 µs an update.

### What limits head B

- The write no longer allocates. Trunk's `markObservers` built two arrays a write, 128 bytes. Head grows the heap by 0.25 bytes a write.
- A round already in creation order no longer sorts. A state's observers live in a `Set`, which iterates in hash order, so one write to a shared state still fills an out-of-order round. Head sorts it by numeric id, which `profile.luau` puts at 0.65 µs a prop under load, against 0.86 for the comparator sort on a quiet machine.
- Every sink call runs under `pcall`, which costs about 0.25 µs a prop over a bare property write (0.73 against 0.49 under load). Vide calls its effect directly at the write.
- The flush's per-entry bookkeeping (`queued`, `mark`, the per-effect run count) is the rest of the update gap.
- A binding is a table with 15 named fields, so Luau gives it a 16-slot hash, about 570 bytes. Head spends 583 bytes a reactive prop and Vide 497. Fewer than 9 fields would halve the table, and the fields the flush and ownership share with effects already exceed that.

## Round 1

A measurement spike, not a library. Should Crucible lower JSX to Instance code on its own reactive graph, or keep Vide? This folder holds hand-written Luau for what a lowering would emit, in two variants, and the same UI written the Vide way. All three run on one fake Instance in Lune.

Run from the repository root after `pnpm build`:

```sh
lune run spike/ui/check          # the three variants behave the same; B keeps its guarantees
lune run spike/ui/bench 15       # every section; `lune run spike/ui/bench 15 4` runs one
```

## Files

- `lib/runtime.luau` loads a fresh Crucible reactive core and a fresh Vide at optimization level 2. It edits one line of the compiled flush, so a node marked `binding` goes to `runBinding` instead of `runOnce`.
- `lib/instance.luau` is the fake Instance: a userdata proxy whose property writes go through `__newindex`.
- `lib/binding.luau` is variant B's binding node.
- `lib/crucible-ui.luau` is the lowered UI for variants A and B, with `Show` and a keyed `For`.
- `lib/vide-ui.luau` is the same UI in Vide.
- `lib/gc.luau` forces collection. Lune's Luau accepts only `collectgarbage("count")`, so it allocates garbage until a weakly held sentinel dies, repeats until the low holds steady, and takes the lowest count.
- `vendor/vide` is Vide 0.6.1, verbatim, under its MIT licence. Strict mode and class defaults are off, so all three variants write the same properties.

## The two lowerings

Fixed props are assigned at creation. Components are plain functions that run once in their caller's owner. The root Instance of a mount, a `Show` branch or a `For` row is owned with `own`; Instances under it go with it. `Show` and `For` drive their branches from a general `effect` in both variants, because they run component code. Each branch or row is a scope with an explicit parent, so it outlives the driver run that made it.

- **A**: an initial read, then `effect(() => instance[prop] = read())` per prop.
- **B**: `bind(instance, prop, source)`. The source is a state, or a computed the compiler wraps around any other expression. The node has the shape of an effect node, so the flush queues it, sorts it by id, bounds it and validates its computed source as it would an effect. Its owner retires it the same way an effect is retired. Its run validates the source, reads the value and does a `pcall`ed property write. That costs no worker, scope or run lifetime, no dependency tracking (one fixed edge) and no cleanup slot.

### Guarantees B drops, and why compiler output can drop them

1. **Yield and error isolation of a user body.** B has no user body. A yield or throw in the expression lives in its computed, which runs in its own boundary and caches the failure. B stops on a cached failure and reports it, as an effect would.
2. **Dynamic dependencies.** A binding reads only its one source. A conditional read like `a() ? b() : c()` becomes a computed, which tracks dynamically.
3. **The run lifetime.** A property write cannot create effects, scopes or cleanups, so there is nothing to own per run.
4. **Luau running inside the write.** B assumes Deferred signal behaviour, which Crucible places set. Under Immediate mode, a `Changed` listener would run on the flush thread, and a yield there would suspend the flush. The write is under `pcall`, so a throw stops only that binding.

B keeps: running on Crucible's flush and never at the write, the flush's creation order and bounds, unsubscribing when the owner ends, and a stop with a diagnostic on failure. `check.luau` asserts each of these.

## Results

Lune 0.11, optimization level 2, 15 runs, 1,000 elements (10,000 for memory). Medians, with min to max in brackets.

| Test | Crucible A | Crucible B | Vide |
| --- | --- | --- | --- |
| Mount, µs an element | 59.1 (47.6–85.9) | **15.0** (11.9–19.3) | 23.6 (17.8–36.4) |
| Unmount, µs an element | 9.1 (6.9–12.2) | 5.0 (3.5–9.9) | **2.8** (2.2–3.8) |
| Update, 1,000 separate states, µs a prop | 31.1 (25.7–33.6) | 7.4 (5.2–15.2) | **4.8** (3.8–8.1) |
| Update, 1,000 bindings on one state, µs a prop | 28.3 (22.7–43.5) | 6.1 (4.4–11.4) | **3.7** (2.7–7.5) |
| For: reverse, ms | 32.3 (28.8–42.4) | 9.4 (7.8–21.5) | **5.9** (4.4–19.9) |
| For: insert 100 in the middle, ms | 27.1 (19.8–32.9) | 9.1 (8.2–15.6) | 9.6 (6.4–23.8) |
| For: remove 100, ms | 15.0 (12.0–22.2) | 5.8 (4.7–9.5) | 5.6 (3.8–6.5) |
| For: swap two rows, ms | 1.14 (0.82–1.50) | 1.13 (0.82–3.55) | **0.74** (0.30–1.16) |
| Memory, bytes an element, Instance included | 3,738 (2,827–4,227) | 2,196 (2,071–2,274) | **1,451** (1,353–2,005) |

The memory noise floor is 31 bytes an element (−24 to 84). The fake Instance alone costs 390 bytes. That leaves about 1,670 bytes a reactive prop for A, 900 for B and 530 for Vide.

Breakdown of a B update (section 5), in µs a prop: the general state write (mark and queue) costs 3.35 with separate states and 1.37 with one state. The flush costs 3.8 to 4.0. The flush's sort costs up to 1.4 of that when the queue comes out of order, as one state's observer set does. A bare fake property write costs 0.56, and 0.84 under `pcall`. Vide's whole write plus effect run costs 4.7 with separate sources and 3.7 with one.

Section 6: a Crucible-to-Vide bridge built on B's node costs 8.9 µs to create and 11.0 µs an update. PR #69's `watch`-based bridge costs 77 and 48.

`.lune/bench-effects.luau` on this branch: 22.5 µs an effect rerun (16.3–27.3) and 36.7 µs with a cleanup (29.3–46.2).

Lune overstates coroutine and global-lookup costs compared with Roblox, where Luau has native codegen and faster builtin access. A's coroutine-heavy path is the most inflated. B's update path uses no coroutines, so its gap to Vide comes mostly from table allocation and calls, which Lune does not inflate as much.
