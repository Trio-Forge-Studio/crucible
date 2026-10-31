# Module engine probes

These plain Luau instruments measure Roblox behavior for the networking, reactive, and data contracts. S0 remains incomplete. Question 1c has no surviving-client observation. Questions 5 and 6 are parked on unpublished-place API errors.

The full repository check exits 1. It builds the sample, then reports 243 Lune cases passed and two existing consumer-compilation cases failed because roblox-ts cannot find compiler-types and types. See [check output](results/repository-check.log). Their files are outside this unit's write scope.

## Rerun

Use Windows with Studio signed in and Assistant MCP enabled. Run from the repository root.

```powershell
pnpm install --pm-on-fail=download
rokit install
node probes/modules/run.mjs Deferred short
pnpm exec vp fmt probes/modules
node --test probes/modules/verification.test.mjs
node probes/modules/mutate.mjs
pnpm check
```

Archive existing `deferred-short*` results before rerunning. The driver replaces them. It builds under the already ignored `build/modules/`. Each place name includes a UUID and driver PID. It selects only that exact MCP name, rejects ambiguity, and checks the owned PID's command line. It logs free physical memory and refuses a launch while a previously recorded PID exists.

The first Play records streaming, catalog, handler order, deferred callbacks, and API errors. It waits 65 seconds after client completion for config reads, fires the handler kick, observes five seconds, and stops. The second Play sets the startup-kick attribute and observes 15 seconds. The driver then saves console output, the native log, extracted S0 rows, and session metadata. It calls CloseMainWindow on its PID, waits up to 30 seconds, then uses Stop-Process only on that PID if needed. Check `processRemaining` in the session record.

Run one driver at a time. If startup crashes, preserve the native log and session, wait 60 seconds, and retry with at most three launches total. A failed MCP connection is a blocker. Do not select another place.

## Results

All measured rows use Deferred SignalBehavior and Studio 0.745.0.7451137. These are version-specific observations. Line ranges refer to the extracted log. `fN` counts a local Heartbeat callback, not an engine frame identifier.

| Question | Answer and contract | SignalBehavior | Studio version | Evidence |
| --- | --- | --- | --- | --- |
| 1a | NET 67, 68. The distant part was absent locally and its field arrived nil. String and number siblings survived. FireClient succeeded. This part never streamed in; a previously visible part streaming out remains unmeasured. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L52-54, L486-486 |
| 1b | NET 67. The client-created part field arrived nil on the server. Its sibling survived. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L54-54 |
| 1c | Unmeasured for surviving clients. FireAllClients ran inside PlayerRemoving with Parent=nil. Local Play had only the departing player and logged no left-packet delivery. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L1008-1013, L1502-1503 |
| 2a | NET 12, 13. Published=true was readable immediately after WaitForChild returned. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L37-39 |
| 2b | NET 12. Both 867 and 66,559 ASCII bytes arrived intact with the exact lengths. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L38-39 |
| 2c | The fallback search was unnecessary because 66,559 bytes succeeded. That is the largest tested size, not an established maximum. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L39-39 |
| 3a | NET 48. All 200 starts followed 1 through 200. The client sent them in one non-yielding loop. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L55-485 |
| 3b | NET 52. Handler 1 yielded for one second. Starts 2 through 200 ran before handler 1 ended. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L55-485 |
| 3c | NET 77. FAIL arrived in both startup and OnServerEvent cases. PlayerRemoving ran after each Kick returned. This does not establish guaranteed delivery. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L1008-1013, L1032-1062, L1502-1503 |
| 3d | NET 35, 52. PlayerRemoving ran after the handler's Kick returned and before the yielding callback returned. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L1008-1013 |
| 4 | RX 31. In 32 complete client intervals the order was Heartbeat, its defer, deferred signal handler, its defer, PreRender, its defer, PreAnimation, PreSimulation, PostSimulation. In 30 server Heartbeat intervals containing remote handlers, their defers ran after the handlers and before PreSimulation, with the same counter. One interval contained two handlers before either defer. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L6-1006 |
| 5a | Parked. No developer product was named. Product enumeration on gameId=0, placeId=0 returned HTTP 404 (Not Found). No purchase or held receipt ran. Grants and duplicates remain unknown. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L44-44 |
| 5b | Parked for the same product failure. PlayerAdded ran but no unacknowledged receipt was available. Receipt ordering remains unknown. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L7-44 |
| 6a | Parked. String, JSON text, and table testing overrides were accepted, but GetConfigAsync returned ConfigService: Failed to load config. No GetValue return type was measured. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L1-4, L1007-1007 |
| 6b | Parked. No snapshot was obtained, so Outdated was not measured. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L1007-1007 |
| 7a | Documented only. The examined pages state no in-engine request quota or complete allowed-character key grammar. Cloud management limits key length to 256 characters and repositories to 100 keys. The experience guide separately lists 1,000 active configs. These are different contexts. | Not applicable | Not applicable | [documentation record](results/documentation.md) L1-9 |
| 7b | DATA 2. SetTestingValue accepted S0.Dotted without error. GetConfigAsync failed, so retrieving a configured dotted key remains unmeasured. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L4-4, L1007-1007 |

## Evidence checks and limits

`verification.test.mjs` checks NET 67's missing slots, NET 12 and 13's catalog observations, NET 48 and 52's ordering and yielding, RX 31's observed order in at least 30 complete render intervals, NET 35 and 77's kick sequence, and DATA 58, 97 and 100's environment failures. It also checks unique launch names and exact session selection. These validate the recorded observations. Rerun Studio to detect engine changes.

`mutate.mjs` changes a disposable trace copy in `build/modules/`. All six mutations caused the verifier to exit 1. The runner exited 0. See [mutation output](results/mutations.log).

Question 1c needs a multi-client run. The available MCP start tool exposes local Play only. This unit did not launch extra Studio processes under the one-Studio rule. A permitted multi-client mode is needed before that acceptance item closes. The script already broadcasts from PlayerRemoving.

Question 1a measures a streaming-excluded part that was never visible. A previously visible part streaming out still needs an observation. The startup kick runs on PlayerAdded, not inside a Crucible boot drain. Successful preceding FAIL delivery remains best-effort evidence.

No measured answer establishes a contract contradiction. DATA 58's hold and DATA 97 and 100's decode and refresh remain unverified. No approved contract changed. Build the Lever produced the driver and mutation runner. Model the Domain kept ownership in the session record. Prove It Works required Studio traces and explicit gaps.

The initial run precedes the kick extension. The startup-only run checks driver connectivity. All three successful launches retain separate native logs and session records. None crashed. All owned PIDs exited after the recorded CloseMainWindow timeout and PID-specific Stop-Process. No other Studio was driven or closed.
