# Asks model

`Asks.tla` models one client's asks over Crucible's network session: `Network.ask` on the client, the client's pending asks and their timers, its queue before READY, the server's handlers with their lifetime and their count toward `MAX_ACTIVE_HANDLERS`, the retirement of the Subsystem that receives the ask, and either side ending the session. `packages/crucible/src/internal/network/client-session.ts`, `server-session.ts` and `session.ts` implement it.

Run it with `cargo xtask tla`. The command needs Java 11 or later on `PATH` or in `JAVA_HOME`. It downloads TLC 1.7.4 into `target/tla` and checks the jar's SHA-256, or uses the jar `TLA2TOOLS_JAR` names.

## States

An ask is `pending[n]`: `none` before the caller asks, `waiting` until its reply, its timeout or the session's end, and `settled` once its caller has its result. `ClientSession` keeps exactly the waiting asks in its pending map, and each entry's `ask` is the `Ask` union: `Waiting` with its timer, or `Settled` with its result. A late or second reply, timer or end finds no waiting entry and changes nothing.

A request on the server is `handler[n]`: `running` while it counts toward the cap, `overrun` once it has outlived `HANDLER_LIFETIME_SECONDS` and stopped counting, and `done` when it returns. Its reply is open until the handler or the retirement of its Subsystem sends it, and then sent. `Running` in `session.ts` holds the `Handler` enum and the `Reply` union, whose `None` variant is an event's, which never replies.

`Network.ask` returns at once with `disconnected` when the session ended and `overloaded` when `MAX_PENDING_ASKS` asks wait, and a queue overflow before READY fails the session, which settles every waiting ask, the new one included, before `ask` returns. The server refuses a request it cannot start with one reply: `invalid` when its payload does not decode, `unavailable` when no claim receives it, and `overloaded` at the cap.

## What TLC checks

| Property | In plain English | TLA+ |
| --- | --- | --- |
| `OneResult` | Each ask's caller gets one result: its reply, its timeout or the session's end, never two, and never a late reply after a timeout. | `\A n \in Nonces : resumed[n] <= 1` |
| `Accounted` | Every ask the caller made is waiting or has its result, and an ask holds a result exactly when it has settled. | `(n <= asked) <=> (pending[n] # "none")` and `(pending[n] = "settled") <=> (result[n] # "none")` for each `n` |
| `Bounded` | The client never holds more waiting asks than `MAX_PENDING_ASKS`. | `Cardinality(Waiting) <= MaxPending` |
| `OneReply` | The server replies to each request at most once, though both its handler and its Subsystem's retirement may try. | `\A n \in Nonces : replies[n] <= 1` |
| `Counted` | While the peer lives, its count is exactly its handlers within their lifetime, and never above `MAX_ACTIVE_HANDLERS`. | `speer # "ended" => counted = Cardinality({n : handler[n] = "running"}) /\ counted <= MaxActive` |
| `Settles` | Every ask that waits gets its result, even when the client never commits and its packet stays queued. | `\A n \in Nonces : pending[n] = "waiting" ~> pending[n] = "settled"` |

`OneResult` and `Settles` together say an ask gets exactly one answer or a timeout, never both. `resumed` and `replies` are ghost variables: the code keeps neither, and the properties count with them.

`Asks.cfg` checks three asks, two pending at most, one packet queued before READY and one active handler. Packets and timers are weakly fair; asks, the client's commit, handler returns, lifetimes, retirement and either side ending are not. TLC 1.7.4 explores 664,595 states, 195,685 of them distinct, to depth 22, in under a minute on two workers.

## Assumptions

- **A1.** The RemoteEvent delivers each side's packets in order and never twice, and drops the packets in flight when a side ends. Roblox documents in-order delivery for one RemoteEvent.
- **A2.** One client sends nonces in increasing order. The server drops a request whose nonce is not above the last it accepted, without a reply, so a client that broke A2 would wait for its timeout. The model's client never breaks it.

The model numbers every ask, those refused at once included, where the code numbers only the asks it sends, which changes no property. It leaves out events and `ClientReady` claims, which reply nothing, decoding and encoding, which `network-codec.luau` and `network-envelope.luau` cover, a reply naming the wrong message, which fails the session as `malformed` like any other violation, and the receive batching that resumes callers only after a packet's dispatch returns, which changes when a caller resumes and not what it gets.

## Each property catches its bug

`mutants.toml` holds the mutants. Each changes one part of the model, and `cargo xtask tla` fails unless TLC reports a counterexample to the property that mutant names.

| Mutant | Bug | Counterexample |
| --- | --- | --- |
| `late-reply-settles` | The client settles an ask on any reply, without checking that it still waits. | `OneResult`, 7 states |
| `settle-keeps-the-entry` | Settling an ask leaves it in the pending map, so its timer or a reply settles it again. | `OneResult`, 4 states |
| `timer-spares-a-queued-ask` | The timeout leaves an ask alone while its packet waits for READY. | `Settles`, a lasso |
| `pending-cap-ignored` | `Network.ask` adds a pending ask past `MAX_PENDING_ASKS`. | `Bounded`, 5 states |
| `retire-leaves-the-entry-open` | A retirement answers a running request without settling its entry, so the handler replies again. | `OneReply`, 7 states |
| `finish-ignores-the-settled-entry` | A finishing handler replies although its entry was already answered. | `OneReply`, 7 states |
| `overrun-keeps-counting` | A handler past its lifetime still counts toward the cap. | `Counted`, 6 states |
| `finish-uncounts-twice` | A handler that overran is uncounted again when it returns. | `Counted`, 7 states |
| `no-active-cap` | The server starts a handler past the cap. | `Counted`, 7 states |
