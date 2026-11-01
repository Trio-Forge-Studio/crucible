# Module engine probes

These plain Luau instruments measure Roblox behavior for the networking, reactive, and data contracts. Question 1c remains parked because local Play supplies no surviving second client. Questions 5a, 5b, 6a and 6b ran in the user's published test place in the r3 run. Their answers are in the rows below.

## Rerun

Use Windows with Studio signed in and Assistant MCP enabled. Run from the repository root.

```powershell
pnpm install --pm-on-fail=download
rokit install
node probes/modules/run.mjs Deferred short r2
pnpm exec vp fmt probes/modules
node --test probes/modules/verification.test.mjs
node probes/modules/mutate.mjs
pnpm check
```

Archive existing `deferred-short-r2*` results before rerunning. The driver replaces them. Its optional third argument supplies a result suffix. It builds under the already ignored `build/modules/`. Each place name includes a UUID and driver PID. It selects only that exact MCP name, rejects ambiguity, and checks the owned PID's command line. It logs free physical memory and refuses a launch while a previously recorded PID exists.

The first Play records streaming, catalog, handler order, deferred callbacks, and API errors. For the streaming transition, the client waits for PreviouslyVisiblePart, then asks the server to move its character to 10,000 studs. StreamOutBehavior is Opportunistic, with minimum radius 64 and target radius 128. The client retains its Luau reference until it observes Parent=nil, then asks the server to send that same part. The server logs that the part still exists in Workspace. The driver waits for the payload observation before the handler kick. It also waits 65 seconds after client completion for config reads, observes the kick for five seconds, and stops. The second Play sets the startup-kick attribute and observes 15 seconds. The driver saves console output, the native log, extracted S0 rows, and session metadata. It calls CloseMainWindow on its PID, waits up to 30 seconds, then uses Stop-Process only on that PID if needed. Check `processRemaining` in the session record.

Run one driver at a time. If startup crashes, preserve the native log and session, wait 60 seconds, and retry with at most three launches total. A failed MCP connection is a blocker. Do not select another place.

## Published-place run

`published.mjs` drives questions 5 and 6. It needs a signed-in Studio with Assistant MCP enabled. It launches its own Studio on place 101113691364993 and universe 10774131158, and never publishes over the place. The probe `published.server.luau` is inserted into the open session and destroyed before the Studio closes.

```powershell
$env:G1_EXCLUDE_STUDIO_IDS = "<MCP ids of any Studio you already have open>"
node probes/modules/published.mjs r3
```

The driver baselines the MCP studio list, launches by PID and finds its own native log from the launch arguments. It proves a candidate session is its own by printing a marker that must appear in that log, and sends nothing else to an unproven session. It plays five times: three purchases held 5, 60 and 120 s, a purchase answered NotProcessedYet, a join that assigns ProcessReceipt 15 s late, a second NotProcessedYet purchase, and a join that assigns it at once.

The Buy and OK buttons sit at fixed pixels of the maximized Studio window. `window.ps1` samples that window alone, never the desktop, and the driver clicks with the MCP mouse tool at the window offset (295, 274). When the pixels do not match, the run ends with the question marked `needs-human-click` in the session record. Archive existing `published-r3*` results before rerunning.

## Results

Rows 1 to 4 and 7 use Deferred SignalBehavior. Rows 5 and 6 ran in the published place under its own SignalBehavior. All rows use Studio 0.745.0.7451137. These are version-specific observations. Line ranges refer to the extracted log. `fN` counts a local Heartbeat callback, not an engine frame identifier.

| Question | Answer and contract | SignalBehavior | Studio version | Evidence |
| --- | --- | --- | --- | --- |
| 1a | NET 67, 68. The never-visible distant part arrived nil. In r2 the client saw PreviouslyVisiblePart, then observed its removal after character relocation. Workspace lookup was nil at reception, but the payload field contained PreviouslyVisiblePart. The client held a Luau reference across removal. Both siblings survived and FireClient succeeded. Stream-out did not make this slot absent in this run. | Deferred | 0.745.0.7451137 | [r1 trace](results/deferred-short.log) L52-54, L486; [r2 trace](results/deferred-short-r2.log) L558, L576-577, L1010-1013 |
| 1b | NET 67. The client-created part field arrived nil on the server. Its sibling survived. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L54-54 |
| 1c | Parked for surviving clients. FireAllClients ran inside PlayerRemoving with Parent=nil. Local Play had only the departing player and logged no left-packet delivery. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L1008-1013, L1502-1503 |
| 2a | NET 12, 13. Published=true was readable immediately after WaitForChild returned. The remote existed before the client looked, so WaitForChild never waited. NET 13's waiting path, a client that waits during a slow server boot, was not exercised. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L37-39 |
| 2b | NET 12. Both 867 and 66,559 ASCII bytes arrived intact with the exact lengths. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L38-39 |
| 2c | The fallback search was unnecessary because 66,559 bytes succeeded. That is the largest tested size, not an established maximum. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L39-39 |
| 3a | NET 48. All 200 starts followed 1 through 200. The client sent them in one non-yielding loop. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L55-485 |
| 3b | NET 52. Handler 1 yielded for one second. Starts 2 through 200 ran before handler 1 ended. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L55-485 |
| 3c | NET 49, 76, 77. FAIL arrived in both startup and OnServerEvent cases. This does not establish guaranteed delivery. PlayerRemoving ran after each Kick returned, 1.03 s later for the handler kick in r1 and 1.98 s in r2, and 3.12 s later for the startup kick in r1 and 2.86 s in r2. Between the startup Kick and PlayerRemoving the server kept running handlers for the kicked player's packets, 409 rows in r1 and 407 in r2, so the contract must end a failed peer when the failure happens and drop its later packets (NET 49, 76) rather than wait for PlayerRemoving, as it already does. | Deferred | 0.745.0.7451137 | [r1 trace](results/deferred-short.log) L1008-1013, L1032-1062, L1502-1503; [r2 trace](results/deferred-short-r2.log) L1015-1020, L1046-1047, L1519 |
| 3d | NET 34, 35, 52, 76, 77. PlayerRemoving ran 1.03 s (r1) and 1.98 s (r2) after the handler's Kick returned. It came before the handler returned only because the handler's 2 s yield outlasted that delay. In r2 the yield ended 0.066 s after removal. NET 34 forbids a ClientReady callback from yielding, so user code that kicks inside a ClientReady callback returns before PlayerRemoving fires (NET 35). The peer can end during that callback only through Crucible's own synchronous failure path (NET 76, 77). | Deferred | 0.745.0.7451137 | [r1 trace](results/deferred-short.log) L1008-1013; [r2 trace](results/deferred-short-r2.log) L1015-1020 |
| 4 | RX 31. In 32 complete client intervals the order was Heartbeat, its defer, deferred signal handler, its defer, PreRender, its defer, PreAnimation, PreSimulation, PostSimulation. In 30 server Heartbeat intervals containing remote handlers, their defers ran after the handlers and before PreSimulation, with the same counter. One interval contained two handlers before either defer. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L6-1006 |
| 5a | DATA 57, 58. Studio test purchases of product 3726289768 honoured ProcessReceipt callbacks held for 5, 60 and 120 s. Each callback ran once and returned PurchaseGranted after holding 5.017, 60.014 and 120.012 s. The completion dialog and PromptProductPurchaseFinished followed. No duplicate callback arrived for any purchase id. Holds above 120 s were not tried. The Buy and OK clicks went through the MCP mouse tool. | Place default, not logged | 0.745.0.7451137 | [trace](results/published-r3.log) L10-28 |
| 5b | DATA 58. Not measurable in Studio. A receipt returned NotProcessedYet was never offered again: not in the same play for 100 s, not at the next join with ProcessReceipt assigned at once, and not when assigned 15 s after PlayerAdded. Studio test purchases leave no unacknowledged receipt to redeliver, so the join-time order against PlayerAdded stays unmeasured and needs a live server. | Place default, not logged | 0.745.0.7451137 | [trace](results/published-r3.log) L40-63, L64-83, L84-107, L108-127 |
| 6a | DATA 97. GetValue("GlobalEvent") on the JSON dashboard config returns a table, `{"endsAt":0,"multiplier":2}`, with a numeric multiplier. It is not a string. A missing key returns nil. A JSON string set with SetTestingValue stays a string, and GetConfigAsync returns it as typeof string. | Place default, not logged | 0.745.0.7451137 | [trace](results/published-r3.log) L6-8, L35 |
| 6b | DATA 98, 100. Outdated stayed false for 300 s idle. SetTestingValue made it true at once on the held snapshot and on a freshly fetched one, and UpdateAvailable fired. The held snapshot kept reading its old table until Refresh, which cleared Outdated. A dashboard edit could not be driven from Studio, so what turns Outdated true after a real dashboard edit is `needs-dashboard-change`. | Place default, not logged | 0.745.0.7451137 | [trace](results/published-r3.log) L9, L29-37 |
| 7a | Documented only. The examined pages state no in-engine request quota or complete allowed-character key grammar. Cloud management limits key length to 256 characters and repositories to 100 keys. The experience guide separately lists 1,000 active configs. These are different contexts. | Not applicable | Not applicable | [documentation record](results/documentation.md) L1-9 |
| 7b | DATA 2. SetTestingValue accepted S0.Dotted without error. GetConfigAsync failed, so retrieving a configured dotted key remains unmeasured. | Deferred | 0.745.0.7451137 | [trace](results/deferred-short.log) L4-4, L1007-1007 |

## Evidence checks and limits

`verification.test.mjs` checks NET 67's missing slots, NET 12 and 13's catalog observations, NET 48 and 52's ordering and yielding, RX 31's observed order in at least 30 complete render intervals, NET 35 and 77's kick sequence with at least 1 s from each Kick to PlayerRemoving in r1 and r2, and DATA 58, 97 and 100's environment failures. The r2 spec checks visibility, removal, server presence, successful send, and the retained payload slot in observation order. It also checks unique launch names and exact session selection. These validate the recorded observations. Rerun Studio to detect engine changes.

The r3 specs for DATA 57, 58, 97, 98 and 100 read `published-r3.log`, and eleven mutations in `mutate.mjs` break them; see [r3 output](results/mutations-r3.log). `mutate.mjs` changes a disposable trace copy in `build/modules/`. The six r1 mutations and three r2 streaming mutations each caused the verifier to exit 1. The runner exited 0. See [r1 output](results/mutations.log) and [r2 output](results/mutations-r2.log).

Question 1c needs a multi-client run. The available MCP start tool exposes local Play only. This unit did not launch extra Studio processes under the one-Studio rule. A permitted multi-client mode is needed before that acceptance item closes. The script already broadcasts from PlayerRemoving.

Question 1a now covers both the never-visible part and a previously visible part that streams out. It does not establish the result without a retained Luau reference, or a guarantee of either nil or non-nil delivery. The received Instance was identified only by `tostring`, its Name. Neither its Parent nor its identity with the retained reference was logged. The payload was a table with named keys, not the wire sidecar's `"1".."R"` keys. NET 68 requires optional non-Player Instance positions and does not require every streamed-out position to arrive nil. This observation does not contradict that rule. The startup kick runs on PlayerAdded, not inside a Crucible boot drain. Successful preceding FAIL delivery remains best-effort evidence.

No measured answer establishes a contract contradiction. DATA 58's hold and DATA 97 and 100's decode and refresh remain unverified. No approved contract changed. Build the Lever produced the driver and mutation runner. Model the Domain kept ownership in the session record. Prove It Works required Studio traces and explicit gaps.

The initial run precedes the kick extension. The startup-only run checks driver connectivity. All three successful launches retain separate native logs and session records. None crashed. All owned PIDs exited after the recorded CloseMainWindow timeout and PID-specific Stop-Process. No other Studio was driven or closed.

The r2 launch used PID 7068 and recorded 1,787,724 KB of free physical memory. Its unique place, exact MCP session, version, and close sequence are in [the session record](results/deferred-short-r2-session.json). The driver's immediate CIM check still reported the process after Stop-Process. The later [closure check](results/closure-r2.json) confirms that neither Get-Process nor CIM found PID 7068. The original session record is preserved unchanged. No startup crash occurred.
