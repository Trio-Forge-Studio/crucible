# Sample Studio evidence

Run `node test/studio/run.mjs Deferred short` after `pnpm build`. The driver builds a unique place, selects only its exact MCP name, records memory and version, and closes its owned PID.

This is partial S2 evidence. It does not verify tag changes, Parent removal, the booted attribute value, render order relative to other callbacks, Error output, or timing constants. The driver does not yet implement the requested crash-retry policy.

| Observation | Signal behavior | Evidence |
| --- | --- | --- |
| Store awake precedes controller awake and start. Server boot returns. | Deferred | results/deferred-short.log L1-4 |
| Renderer awake and start run. Client boot returns. | Deferred | results/deferred-short.log L5-7 |
| Five render callbacks run. | Deferred | results/deferred-short.log L8-12 |
| Controller shutdown precedes store shutdown. | Deferred | results/deferred-short.log L13-14 |
| Store awake precedes controller awake and start. Server boot returns. | Immediate | results/immediate-short.log L1-4 |
| Renderer awake and start run. Client boot returns. | Immediate | results/immediate-short.log L5-7 |
| Five render callbacks run. | Immediate | results/immediate-short.log L8-12 |
| Controller shutdown precedes store shutdown. | Immediate | results/immediate-short.log L13-14 |

Studio version was 0.745.0.7451137. Free physical memory before launch was 3,019,420 KB. The session record identifies PID 21600 and confirms `processRemaining=false`. CloseMainWindow did not close it within 30 seconds. The driver then closed it with Stop-Process by PID.

Immediate used the same version with 1,576,176 KB free before launch. PID 7428 required Stop-Process too. Its immediate CIM check still saw the terminating process. A separate CIM and Get-Process check after the driver returned found no process. The session records `processRemainingAfterRecheck=false` and the recheck time.

No performance number is claimed. Deferred shutdown markers occurred about 5.0 and 5.7 seconds after the driver requested Stop. Immediate markers occurred about 8.0 and 8.2 seconds after Stop. These intervals include Studio stop orchestration and do not isolate runtime hook duration.

S2 is blocked on S1 integration. Neither pinned base contains probes/engine. The coordinator must provide that tree before the requested probe commands can join pnpm check within the assigned write scope.
