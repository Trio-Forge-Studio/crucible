# Sample Studio evidence

Run `pnpm build`, then `node test/studio/run.mjs Deferred short` and `node test/studio/run.mjs Immediate short`. The driver creates a unique place, selects its exact MCP name, checks its PID command line, and closes only that PID. PID reuse does not confer ownership. Startup process exits retry at most three times, sixty seconds apart. Other failures do not retry.

The fix-round runs below ran sequentially after all mutation and build processes finished. No other heavy work from this task ran during either Studio session. These runs were recorded under load, not as a baseline. Another project's Fragile Delivery verify Studio, PID 18576, was present in all ten Deferred samples and the first Immediate sample, then absent from the remaining nine. The user's Roblox Player, PID 46632, was present in all ten samples of each run. Neither foreign process was touched. Host CPU, free memory and Roblox PID samples taken throughout each driver run are retained in `results/fix1-deferred-load.json` and `results/fix1-immediate-load.json`.

`observations.mjs` checks the compiled sample against the actual Roblox port. Its client observer requires the StarterPlayer template discovered by boot. Requiring the PlayerScripts clone would execute a second registration. It observes five render intervals, then replaces the renderer hook with a deliberate error.

| Observation | Deferred trace lines | Immediate trace lines |
| --- | --- | --- |
| Game Debug, Info and Warning on both sides. | 1 to 32 | 1 to 32 |
| Deliberate game Error on both sides. | 4 to 33 | 4 to 33 |
| Store, dependent Controller, server boot. | 5 to 8 | 5 to 8 |
| Tag add binds with required Identity and optional Speed fallback. | 12 | 12 |
| Tag removal disables, destroys and unbinds. | 15 | 15 |
| Tag re-add creates a new generation. | 19 | 19 |
| Parent nil unbinds while the tag remains. | 22 | 22 |
| Reparent creates a new generation. | 26 | 26 |
| LocalPlayer CrucibleBooted is true. | 37 | 37 |
| Bind200, Crucible201, PreRender, five intervals. | 39 to 47 | 39 to 47 |
| Deliberate frame fault at Error level. | 48 | 48 |
| Controller shutdown completes before Store shutdown. | 69 to 70 | 69 to 70 |

Line numbers above refer to `results/deferred-short.log` and `results/immediate-short.log`. The matching `-native.log` files retain the original error text, caller tracebacks and spawned-error stacks. Native logs preserve line order and contents except line-ending normalization and trailing whitespace removal.

Deferred used Studio 0.745.0.7451137, owned PID 34636. Its session record confirms `processRemaining=false`. Immediate used Studio 0.745.0.7451137, owned PID 34260. Its session record confirms `processRemaining=false`.

Shutdown elapsed time runs from the driver Stop request to each hook native-log timestamp. It includes orchestration and is an upper bound on the sample hook path, not hook CPU cost. The regression test requires both hooks within 25 seconds.

The following reference values are retained from [main at fce75c1](https://github.com/Trio-Forge-Studio/crucible/blob/fce75c1005d814494c58bd778fab3dd965477554/test/studio/README.md). Their line references identify the main committed traces, not the newly recorded loaded runs.

## Main lifecycle and G4 reference measurements

`measurements.mjs` imports cloned compiled lifecycle modules through Roblox's roblox-ts RuntimeLib. Each case has isolated module state. Pool effects use the real DenseList for membership. No fake engine executes these measurements. Five trials alternate pool, batches and chained requests. Pool trials discard five warmup frames and time 100 Heartbeat frames, each with 1,000 members, 500 disables, 500 enables and one full walk. Heartbeat waiting, setup, module loading, logging and assertions are outside the timed region. Each trial verifies 53,500 joins, 52,500 leaves, 105,000 visits and zero failures, including warmup.

Deferred measured a median 4.377652 ms per pool frame, range 3.442543 to 4.860833 ms. MAX_BATCHES measured median 18.835400 ms per drain, range 13.397100 to 27.775600 ms. MAX_CHAINED_REQUESTS measured median 95.104600 ms per drain, range 85.604600 to 105.479600 ms. Evidence is deferred-short.log L45-59.

Immediate measured a median 4.574201 ms per pool frame, range 4.216626 to 4.825306 ms. MAX_BATCHES measured median 24.318300 ms per drain, range 13.357800 to 24.705000 ms. MAX_CHAINED_REQUESTS measured median 104.249100 ms per drain, range 87.648800 to 132.595800 ms. Evidence is immediate-short.log L45-59.

A separate instrumented trial runs in both modes, excluded from those medians, at L42-44 of each trace. The following profile values are from Deferred. Pool time was 391.152600 ms over 100 frames, with 386.527600 ms in lifecycle requests and 4.625000 ms in list traversal. The split supports lifecycle machinery as the limiter. Attribution within that remainder to scheduling, protected calls and membership work is inferred. Hook bodies accounted for 23.888901 ms including warmup. The batch profile recorded 2,048 hook calls, 13.228600 ms total and 0.190600 ms in hook bodies. The request profile recorded 65,543 hook calls, 95.108100 ms total and 22.171096 ms in hook bodies. The remainder includes transition scheduling, protected calls, list maintenance and cleanup. The fixture supplies a hook wrapper even when no callback exists, so it includes runner calls a class without that hook would skip. External CPU contention and memory pressure also affect wall time.

Each batch drain reaches exactly 1,024 starts, then reports the MAX_BATCHES failure. The batch count guard limits creation, not recursion or the fixture's 2,000-start guard. Each request drain reports the 65,536-request cap, with 65,539 issued requests including teardown. Cross-object requests reset each target's self-transition count, so MAX_CHAINED_REQUESTS ends the cycle before the 100,000-request fixture guard. Both cap cases require exactly one expected failure per trial. These numbers measure core lifecycle work with small instrumented effects. They exclude game hooks, binding scans and complete frame dispatch. They are not a gameplay frame budget guarantee. The approximately 95 ms Deferred request drain is evidence for coordinator gate G4. This unit does not change either limit.

## Measurements recorded under load

| Run | Sampled host CPU | Sampled free physical memory | Launch free memory |
| --- | --- | --- | --- |
| Deferred | 14 to 84% | 287,244 to 3,214,444 KB | 3,156,588 KB |
| Immediate | 4 to 94% | 351,972 to 3,919,860 KB | 3,648,644 KB |

CPU and memory ranges include launch, play and shutdown orchestration. The timestamped JSON samples distinguish these periods. These timings do not establish a regression or a new baseline.

These runs use the measurement method and counted assertions described in the main reference above.

Deferred measured the following medians and ranges across five trials. Evidence is `deferred-short.log` L53 to 67.

| Case | Median | Range |
| --- | --- | --- |
| Pool | 4.429547 ms/frame | 3.292312 to 4.988496 ms/frame |
| MAX_BATCHES | 19.248200 ms/drain | 13.802700 to 23.996500 ms/drain |
| MAX_CHAINED_REQUESTS | 104.297000 ms/drain | 96.889100 to 122.385800 ms/drain |

Immediate measured the following medians and ranges across five trials. Evidence is `immediate-short.log` L53 to 67.

| Case | Median | Range |
| --- | --- | --- |
| Pool | 3.856314 ms/frame | 3.520226 to 5.139770 ms/frame |
| MAX_BATCHES | 14.347800 ms/drain | 13.404200 to 24.559300 ms/drain |
| MAX_CHAINED_REQUESTS | 107.957700 ms/drain | 99.390900 to 114.377600 ms/drain |

A separate instrumented trial in each mode is excluded from those medians. The measured breakdowns below retain lifecycle, traversal and hook-body attribution.

Deferred profile, `deferred-short.log` L50 to 52.

- `S2 profile pool totalMs=434.681699 walkMs=5.332399 lifecycleMs=429.349300 hookBodyMs=30.814300`.
- `S2 profile MAX_BATCHES totalMs=14.171800 hookBodyMs=0.187900 hookCalls=2048`.
- `S2 profile MAX_CHAINED_REQUESTS totalMs=116.802100 hookBodyMs=26.007707 hookCalls=65543`.

Immediate profile, `immediate-short.log` L50 to 52.

- `S2 profile pool totalMs=413.403200 walkMs=4.952700 lifecycleMs=408.450500 hookBodyMs=27.590502`.
- `S2 profile MAX_BATCHES totalMs=14.461500 hookBodyMs=0.246899 hookCalls=2048`.
- `S2 profile MAX_CHAINED_REQUESTS totalMs=115.394500 hookBodyMs=25.426702 hookCalls=65543`.

The pool split identifies lifecycle requests as the measured remainder after list traversal. Attribution within that remainder to scheduling, protected calls and membership work is inferred. Hook-body timing includes warmup in the pool case. The fixture supplies a hook wrapper even when no callback exists, so it includes runner calls a class without that hook would skip. Instrumentation and external host load affect wall time.

Each batch drain reaches exactly 1,024 starts, then reports MAX_BATCHES. The count guard limits creation before the fixture's 2,000-start guard. Each request drain reports the 65,536-request cap with 65,539 issued requests including teardown. Cross-object requests reset each target's self-transition count, so MAX_CHAINED_REQUESTS ends the cycle before the 100,000-request fixture guard. Both cases require exactly one expected failure per trial. These measurements exclude game hooks, binding scans and complete frame dispatch. They are not a gameplay frame budget guarantee. The retained main reference supplies the timing evidence for coordinator gate G4; these loaded runs do not replace it. This unit changes neither limit.

## Regression and mutation checks

`node --test test/studio/verification.test.mjs` checks native logger severity, game caller tracebacks, the unchanged Crucible fault, owned-process identity, exact session selection, ambiguity rejection, startup retries, render order, shutdown evidence and counted cap measurements. It also checks that listing transformer mutants preserves an active campaign's scratch directory.

`node test/lib/logging-mutants.mjs` rebuilds source mutants in isolated scratch and runs each named Lune spec. `test/logging-mutants.log` records 32 killed mutants, each with build exit 0 and spec exit 1. Scratch is removed with Node's `fs.rmSync`.

`node test/sample-mutants.mjs` covers real fixture copying, PID ownership, retry waiting and sample wiring. `cli/crates/crucible/tests/build.rs` covers transformer fingerprint invalidation. Its receipt is `results/sample-mutants.log`. Transformer receipts are `results/transformer-mutants.json` and `results/transformer-mutants.log`. Fix-round boot, binding and scheduler receipts are under `test` with the `-fix1.log` suffix.

The Lune sample fixture includes a tagged Part without required Identity. The compiled lifecycle spec rejects its admission. Its valid Part has Identity fleet-23 and absent Speed, so field substitution and fallback errors remain distinguishable.
