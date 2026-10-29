# Engine probes

These plain Luau prototypes measure engine behavior for Crucible's runtime port. They contain no Crucible code. Keep them as rerunnable instruments after engine updates.

## Rerun

Use Windows with Studio signed in and Assistant's MCP server enabled. Run from the repository root.

```powershell
rokit install
node probes/engine/run.mjs Deferred short
node probes/engine/run.mjs Deferred long
node probes/engine/run.mjs Immediate short
node probes/engine/run.mjs Immediate long
stylua --check probes
selene --config probes/engine/selene.toml probes
pnpm install --pm-on-fail=download
pnpm exec vp fmt probes/engine
pnpm check
```

The driver builds with Rojo under the already ignored `build/engine/`. It uses WMI to launch Studio outside the shell's process tree, following Fragile Delivery's launch mechanics. Each launch uses a place filename containing the driver PID and a fresh UUID. It selects only that exact MCP place name, rejects multiple matches, and verifies its launched PID's command line. Each launch records `Win32_OperatingSystem.FreePhysicalMemory` in KB. It refuses to launch while any previous recorded PID exists. Run one driver at a time.

After Stop Play, the driver observes for eight seconds in the short case or 45 seconds in the long case. It saves console output, the native log, an extracted `S1|` trace, and session metadata. It calls `CloseMainWindow` on its own PID and waits up to 30 seconds. Only if that PID still runs does it call `Stop-Process -Id`. It records which close method ran and verifies that PID exited. No other Studio is driven or closed.

Each invocation replaces that case's results. Archive existing results before rerunning. If Studio crashes during startup, preserve that attempt's session and native log, wait 60 seconds, and retry. Stop after three startup crashes and report every log and memory reading. If Studio cannot connect to MCP, report that blocker. Do not substitute documentation.

The generated Rojo project sets `Workspace.SignalBehavior` and writes the same label to an attribute for trace metadata. Direct script reads of that property failed in this Studio. The initial instrument failure is preserved in `results/instrument-failure-native.log`. The nested Selene standard extends Roblox with `version()`. The root `luau+lune` standard stays unchanged.

Run `node --test probes/engine/verification.test.mjs` to check staged Luau routing, row 1b evidence, unique launch names, and rejection of ambiguous Studio sessions.

## Read the traces

Native prefixes contain UTC timestamps. Probe messages contain side, sequence number, and elapsed `os.clock()` time. Server `fN` counts one Heartbeat connection's executions. It is an observation boundary, not an engine frame ID. A change from f3 to f4 proves that callback ran between messages, not that a whole new frame elapsed. Client rows record callbacks between Heartbeats and count only intervals containing Bind0. Each final run records 35 complete render intervals.

D means Deferred and I means Immediate. Evidence gives inclusive line ranges in extracted raw logs. Every row uses Studio `0.745.0.7451137`. Observations do not establish guarantees across versions.

Render order R, observed in all 35 complete intervals per run, is `Bind0, Bind100, Bind199, Bind200, Bind201, Bind201A, Bind201B, Bind2000, RenderStepped, PreRender-after, PreRender-before`. A and B were bound in that order. PreRender connections were registered before and after the bindings. R includes the earlier PreRender connection running last.

| Question | Measured answer | SignalBehavior | Studio version | Log and lines |
| --- | --- | --- | --- | --- |
| 1a. Parent nil and back | D removed after the write, f3 to f4. HasTag stayed true. Reparent added again, f5 to f6. I removed and added inside each write at f3 and f5. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L4-18; [immediate-short.log](results/immediate-short.log) L3-14 |
| 1b. ServerStorage and back | Neither move emitted added or removed on the server. Initial AddTag emitted added. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L23-39; [immediate-short.log](results/immediate-short.log) L20-33 |
| 1c. Destroy | One removed signal. D after Destroy, f15 to f16. I inside Destroy at f15, with Parent still Workspace in the handler. HasTag read true immediately after Destroy in both. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L40-47; [immediate-short.log](results/immediate-short.log) L34-41 |
| 1d. AddTag then RemoveTag | Added then removed. D both after the writes, f17 to f18. I inside each write at f17. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L51-56; [immediate-short.log](results/immediate-short.log) L46-50 |
| 1e. Created and cloned at nil | No added handler at nil. Clone retained tag. Parenting emitted added for created then cloned. D after both writes, f23 to f24. I inside each write at f23. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L60-80; [immediate-short.log](results/immediate-short.log) L55-74 |
| 2. task.wait() | Check saw suspended, cancel succeeded, final dead. No resumed segment. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L84-92; [immediate-short.log](results/immediate-short.log) L79-87 |
| 2. task.wait(0) | Check saw suspended, cancel succeeded, final dead. No resumed segment. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L93-101; [immediate-short.log](results/immediate-short.log) L88-96 |
| 2. event:Wait() | Check cancelled before deferred event fire. No resumed segment. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L103-111; [immediate-short.log](results/immediate-short.log) L97-106 |
| 2. Resumption deferred before check | T ran one segment after its first yield. Check cancelled at its second yield. Final dead. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L112-121; [immediate-short.log](results/immediate-short.log) L107-116 |
| 2. Resumption deferred after check | Check cancelled first. No resumed segment. Final dead. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L122-129; [immediate-short.log](results/immediate-short.log) L117-124 |
| 2. task.spawn(T) from callback queued first | Other callback resumed T immediately after its first yield. Check cancelled at its second yield. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L130-139; [immediate-short.log](results/immediate-short.log) L125-134 |
| 2. task.cancel running | pcall returned false with exact error `cannot cancel thread`. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L140-143; [immediate-short.log](results/immediate-short.log) L135-137 |
| 2. task.cancel normal | pcall returned false with exact error `cannot cancel thread`. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L144-147; [immediate-short.log](results/immediate-short.log) L140-143 |
| 2. coroutine.close running | pcall returned false with exact error `cannot close running coroutine`. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L148-150; [immediate-short.log](results/immediate-short.log) L146-148 |
| 2. coroutine.close normal | pcall returned false with exact error `cannot close normal coroutine`. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L151-154; [immediate-short.log](results/immediate-short.log) L151-154 |
| 3. Client render callbacks | R above in 35 complete intervals. At priority 201, Bind201, A, B ran in that order in these samples. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L19-141; [immediate-short.log](results/immediate-short.log) L27-150 |
| 3. Client phases | Render callbacks including PreRender, then PreAnimation, PreSimulation, PostSimulation, Heartbeat. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L19-141; [immediate-short.log](results/immediate-short.log) L27-150 |
| 3. Server phases | PreAnimation, PreSimulation, PostSimulation, Heartbeat in every captured interval. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L2-125; [immediate-short.log](results/immediate-short.log) L2-119 |
| 4. Handler timing | D after firing resumption ended, after Heartbeat increment f47 to f48, within the same native-log millisecond. I inside the writes at f47. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L155-163; [immediate-short.log](results/immediate-short.log) L155-163 |
| 4. Attribute then tag | Attribute handler start and end preceded tag-added handler. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L155-163; [immediate-short.log](results/immediate-short.log) L155-163 |
| 4. Defer inside handler | D nested defer followed tag handler and outer defer at f48. No intervening Heartbeat callback. I nested defer preceded outer defer at f48. These logs observe one flush interval; no engine flush ID is exposed here. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L155-163; [immediate-short.log](results/immediate-short.log) L155-163 |
| 5. Three short callbacks | All started before any ended. All finished near requested 1, 3, and 5 seconds. Their yielding intervals overlapped. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L165-170; [immediate-short.log](results/immediate-short.log) L165-170 |
| 5. 40-second callback | All started. The 1- and 3-second callbacks ended. The 40-second callback did not. Watch tick 29 was last. Native drain reported the outstanding callback at about 30 seconds. | D, I | 0.745.0.7451137 | [deferred-long.log](results/deferred-long.log) L165-198; [immediate-long.log](results/immediate-long.log) L165-198 |
| 5. Client BindToClose | pcall returned false with exact error `BindToClose can only be called on the server.` No callback ran. | D, I | 0.745.0.7451137 | [deferred-short.log](results/deferred-short.log) L7-8; [immediate-short.log](results/immediate-short.log) L17-18 |

## Contract contradictions

1. `synthesis-v2/src/internal/lifecycle.ts:280-297` says, "A deferred check cancels it at its next yield". Under both behaviors, `2.defer-before` and `2.spawn-callback` resumed after their first yield, before the check. Cancellation happened at the second yield. The unconditional next-yield claim is false for these queues.
2. `specs/public-api.md` rule 12 and verdict C6 limit remaining code to "up to its next yield" when cancellation is deferred. Read literally as that bound, the same traces contradict it. The broader phrase "can run once more" fits the observed extra segment. The coordinator must settle the wording and cancellation contract before relying on the bound. This unit changes no API.

Evidence is the question 2 rows above. Rule 10 and C4's DataModel departure assumption matched the measured server signals. Rule 2's 30-second shutdown budget matched native drain output. Rule 15's client phase order matched the captured intervals. No other contradiction was established.

## Limits and follow-ups

These observations cover local Play on one Studio version. Live-server shutdown, streaming, unbounded queued resumptions, and equal-priority guarantees remain unmeasured. The event-wait case queues the fire after the check. Signal absence means none occurred in the two-Heartbeat observation window. MCP can report Edit before server shutdown finishes, so cutoff evidence uses the native drain log.

The coordinator should resolve rule 12's bound and correct the lifecycle comment. S2 should add probes to required Luau checks. Future engine updates should rerun all four cases.

Build the Lever kept `run.mjs` as the repeatable instrument. Prove It Works required actual Studio traces. Model the Domain kept PID ownership and evidence in one session record per behavior and shutdown case.

Native cutoff evidence for deferred-long is [deferred-long-native.log](results/deferred-long-native.log) L2862-2923. The outstanding-handler message occurred 30.001995 seconds after callback 3 started.

Native cutoff evidence for immediate-long is [immediate-long-native.log](results/immediate-long-native.log) L2903-2984. The outstanding-handler message occurred 30.037487 seconds after callback 3 started.
