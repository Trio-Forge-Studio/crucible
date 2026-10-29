# Sample Studio evidence

Run `pnpm build`, then `node test/studio/run.mjs Deferred short` and `node test/studio/run.mjs Immediate short`. The driver creates a unique place, selects its exact MCP name, checks its PID command line, records memory and version, and closes only that PID. PID reuse does not confer ownership. `run.mjs` retries only startup process exits, waits 60 seconds between attempts, and stops after three launches. Each crash session and available native log is archived before retry. No actual startup crash occurred in these runs.

`observations.mjs` checks the compiled sample against the actual Roblox port. Its client observer requires the StarterPlayer template discovered by boot. Requiring the PlayerScripts clone would execute a second registration. The observer wraps the sample renderer for five frames, then replaces its frame hook with a deliberate error. Production runtime and transformer implementation are unchanged.

| Observation | Deferred trace lines | Immediate trace lines |
| --- | --- | --- |
| Store, dependent Controller, server boot. | 1-4 | 1-4 |
| Tag add binds with required Identity and optional Speed fallback. | 5-8 | 5-8 |
| Tag removal disables, destroys and unbinds. | 9-11 | 9-11 |
| Tag re-add creates a new generation. | 12-15 | 12-15 |
| Parent nil disables, destroys and unbinds while HasTag stays true. | 16-18 | 16-18 |
| Reparent creates a new generation. | 19-22 | 19-22 |
| LocalPlayer CrucibleBooted is true. | 29 | 29 |
| BindToRenderStep priority 200, Crucible priority 201, plain PreRender, five intervals. | 31-39 | 31-39 |
| Deliberate frame fault at Error level. | 40 | 40 |
| Controller shutdown completes before Store shutdown. | 61-62 | 61-62 |

Trace files are `results/deferred-short.log` and `results/immediate-short.log`. Original error tracebacks are retained in `deferred-short-native.log` L2464-2473 and `immediate-short-native.log` L2450-2459. They include S2ClientObserver, CrucibleObject, boot, scheduler and runner frames. Native logs preserve line order and contents except line-ending normalization and trailing whitespace removal.

Both runs used Studio 0.745.0.7451137. Deferred launched PID 33096 with 3,532,388 KB free. Immediate launched PID 51572 with 4,526,952 KB free. Both session records confirm `processRemaining=false` after CloseMainWindow and Stop-Process by PID. Compiler checks overlapped Deferred and the transformer mutation campaign overlapped Immediate. These are local timing observations under that load, with no performance comparison claim.

Shutdown elapsed time is measured from the driver's Stop request to each hook's native-log timestamp. This includes Studio orchestration and is an upper bound on the sample hook path, not hook CPU cost. The regression test requires both hooks within 25 seconds.

## Real Luau measurements

`measurements.mjs` imports cloned compiled lifecycle modules through Roblox's roblox-ts RuntimeLib. Each case has isolated module state. Pool effects use the real DenseList for membership. No fake engine executes these measurements. Five trials alternate pool, batches and chained requests. Pool trials discard five warmup frames and time 100 Heartbeat frames, each with 1,000 members, 500 disables, 500 enables and one full walk. Heartbeat waiting, setup, module loading, logging and assertions are outside the timed region. Each trial verifies 53,500 joins, 52,500 leaves, 105,000 visits and zero failures, including warmup.

Deferred measured a median 4.377652 ms per pool frame, range 3.442543 to 4.860833 ms. MAX_BATCHES measured median 18.835400 ms per drain, range 13.397100 to 27.775600 ms. MAX_CHAINED_REQUESTS measured median 95.104600 ms per drain, range 85.604600 to 105.479600 ms. Evidence is deferred-short.log L45-59.

Immediate measured a median 4.574201 ms per pool frame, range 4.216626 to 4.825306 ms. MAX_BATCHES measured median 24.318300 ms per drain, range 13.357800 to 24.705000 ms. MAX_CHAINED_REQUESTS measured median 104.249100 ms per drain, range 87.648800 to 132.595800 ms. Evidence is immediate-short.log L45-59.

A separate instrumented trial runs in both modes, excluded from those medians, at L42-44 of each trace. The following profile values are from Deferred. Pool time was 391.152600 ms over 100 frames, with 386.527600 ms in lifecycle requests and 4.625000 ms in list traversal. The split supports lifecycle machinery as the limiter. Attribution within that remainder to scheduling, protected calls and membership work is inferred. Hook bodies accounted for 23.888901 ms including warmup. The batch profile recorded 2,048 hook calls, 13.228600 ms total and 0.190600 ms in hook bodies. The request profile recorded 65,543 hook calls, 95.108100 ms total and 22.171096 ms in hook bodies. The remainder includes transition scheduling, protected calls, list maintenance and cleanup. The fixture supplies a hook wrapper even when no callback exists, so it includes runner calls a class without that hook would skip. External CPU contention and memory pressure also affect wall time.

Each batch drain reaches exactly 1,024 starts, then reports the MAX_BATCHES failure. The batch count guard limits creation, not recursion or the fixture's 2,000-start guard. Each request drain reports the 65,536-request cap, with 65,539 issued requests including teardown. Cross-object requests reset each target's self-transition count, so MAX_CHAINED_REQUESTS ends the cycle before the 100,000-request fixture guard. Both cap cases require exactly one expected failure per trial. These numbers measure core lifecycle work with small instrumented effects. They exclude game hooks, binding scans and complete frame dispatch. They are not a gameplay frame budget guarantee. The approximately 95 ms Deferred request drain is evidence for coordinator gate G4. This unit does not change either limit.

## Regression and mutation checks

`node --test test/studio/verification.test.mjs` checks owned-process identity, exact session selection, unique names, ambiguity rejection, crash retry policy, probe check wiring and both committed Studio traces. `node test/sample-mutants.mjs` mutates source and rebuilds sample TypeScript for compiled lifecycle cases. Node code is syntax checked and executed directly. Each source is restored in a finally block. The campaign covers fingerprint invalidation, real fixture copying, PID ownership, retry waiting and sample wiring. Its receipt is `results/sample-mutants.log`.

The Lune sample fixture also includes a tagged Part without required Identity. The compiled lifecycle spec rejects its admission. Its valid Part has Identity fleet-23 and absent Speed, so field substitution and fallback errors remain distinguishable.
