# Profile model

`Profile.tla` models one player's profile under a profile store: the lease each server's session takes in the DataStore record, the local authority a session holds while its safe bound lasts, the owned writes that renew the lease and carry the session's data, the release that ends it, Roblox's `ProcessReceipt` callbacks that grant developer products into it, and the one-time legacy import that runs before a session turns writable. `packages/crucible/src/internal/data/profiles.ts`, `session.ts`, `lease.ts` and `receipts.ts` implement it.

Run it with `cargo xtask tla`. The command needs Java 11 or later on `PATH` or in `JAVA_HOME`. It downloads TLC 1.7.4 into `target/tla` and checks the jar's SHA-256, or uses the jar `TLA2TOOLS_JAR` names.

## States

A session is `sess[t]`, one per token. Its `phase` is the `Authority` union in `session.ts`:

- `loading` until its acquire's answer activates it or ends the load. A load the player leaves ends at once and owes the write that removes its own lease, if its acquire took one.
- `importing` when the acquired record lacks the import's mark. The import returns nothing, a record that admission takes, or one it refuses. A refused record, a passed safe bound or a player who leaves ends the load as `loading` does; otherwise the session turns `active` at revision 1, with the mark and its first write due. The safe bound is the one its acquire set.
- `active` while it holds the lease. It is writable exactly while `now` is before its safe bound, which an activation sets and each confirmed write extends, from the earliest send of the request that may have stored it plus `LOCAL_AUTHORITY_SECONDS`.
- `sealed`, with the reason, once the player leaves, its safe bound passes, or a write finds the lease gone. Writes stop at once, and the session sends its release.
- `ended`, with the release's outcome: `committed`, `fenced` or `given up`.

A purchase in a session is `Receipt` in `session.ts`: staged with its effects when the grant ran, durable once a confirmed write's record holds its id, and durable without effects once `PurchaseGranted` ran them. In the model, a session's `staged`, `durable` and `effects` sets hold the same facts.

The record, `store`, holds the lease's token, epoch and expiry, the commit marker of the last owned write (its token, revision and op), the payload, the receipts and the import's mark. The payload counts the grants of each purchase, the times the legacy record was imported into it (`Legacy`) and whether it holds an import result admission refused (`Bad`). A session's snapshot carries its mark, and an owned write or a carrying acquire stores it with the payload, in the same write, and never clears it. An acquire takes the lease when it is absent, expired past `TAKEOVER_GRACE_SECONDS`, or held by one of this server's own sealed sessions whose release settled. Then it carries that session's snapshot when the record does not already hold it. An owned write lands only for the token and epoch that hold the lease, before its expiry, and a retry whose op already landed reads `already`.

## What TLC checks

| Property | In plain English | TLA+ |
| --- | --- | --- |
| `OneAuthority` | At most one session is writable at any moment, across every server. | `Cardinality({t : Writable(t)}) <= 1` |
| `GrantOnce` | No purchase is granted twice, in the record or in any session's data. | `store.payload[p] <= 1` and `sess[t].value[p] <= 1` |
| `GrantWithId` | A grant and its purchase id are stored in one revision: the record holds one exactly when it holds the other. | `(p \in store.receipts) <=> (store.payload[p] >= 1)` |
| `AckedDurable` | Roblox is answered `PurchaseGranted` only for a purchase whose grant the record holds, and the record keeps holding it. | `\A p \in acked : store.payload[p] = 1 /\ p \in store.receipts` |
| `DurableStored` | What an active or sealed session believes durable, the record holds. | `sess[t].durable \subseteq store.receipts` |
| `EffectsOnce` | `afterGrant` effects run at most once per purchase, and only once Roblox was told it is granted. | `ran[p] <= 1 /\ (ran[p] = 1 => p \in acked)` |
| `ImportOnce` | The legacy record is imported at most once, and no import starts once the record has held the import's mark. | `~rerun /\ store.payload[Legacy] <= 1 /\ sess[t].value[Legacy] <= 1` |
| `NoWriteAfterLoss` | A session whose import lost authority, by its safe bound or by the player leaving, never sends an owned write, so nothing the import read is written. | `req[s].kind = "own" => req[s].tok \notin lost` |
| `AdmittedBeforeWritable` | An import result that admission refuses never reaches a writable session or the record. | `store.payload[Bad] = 0 /\ sess[t].value[Bad] = 0` for active and sealed sessions |
| `StopsWriting` | A session's data changes only while it is writable: a sealed, ended or expired session never writes. | `[][sess'[t].value # sess[t].value => Writable(t)]_vars` |

`ran`, `rerun` and `lost` are ghost variables: the code keeps no count of the effects it ran, no record of an import that started after the mark was stored, and no set of the imports that lost authority. `EffectsOnce`, `ImportOnce` and `NoWriteAfterLoss` read them.

`Profile.cfg` checks two servers, one purchase, two sessions, one lost answer, a lease of 3, local authority of 2 and a grace of 0, with the clock stopping at 4. The player starts on one server and may teleport to the other once. TLC 1.7.4 explores 44,063,823 states, 11,609,735 of them distinct, to depth 50, in about ten minutes on two workers.

## Assumptions

- **A1.** Every server reads one clock. The code's `TAKEOVER_GRACE_SECONDS` and the margin between `LEASE_SECONDS` and `LOCAL_AUTHORITY_SECONDS` absorb skew between servers' clocks. The model sets the grace to 0 and keeps the margin at one tick, so it checks the protocol with none of the skew allowance left over.
- **A2.** A server sends one request for the player's key at a time, which the scheduler's per-key lane gives. A load that the player leaves stops waiting, and its acquire may still land.
- **A3.** A request lands in the DataStore atomically, at some time after it was sent. Its answer can be lost after it landed or before, and the code learns nothing from a lost answer but that it may have landed.
- **A4.** Roblox calls `ProcessReceipt` for a purchase it has not been told is granted, on the server the player is on, and again later until one call answers `PurchaseGranted`.

## What the model leaves out

- What the import reads and how it retries. Each `Import` step returns one of the three results, and the model does not stop the import's own work: the code drops a late result rather than cancel the thread.
- `onLoad` and migrations between the import and activation. `Bad` stands for any result that admission or a migration refuses.
- Handoff between servers, Subsystem retirement, shutdown and the inbox. Each seals a session for a reason the model's `Leave` already covers, and `data-handoff.luau`, `data-shutdown.luau` and the inbox specs cover their own rules.
- The gap between stopping a load and the load seeing it. `Leave` abandons a loading or importing session and ends its load in one step, so `ReplyAcquire` and `Import` never meet an abandoned session that goes on. The code marks the session `abandoned` and ends the load when it next looks, which can be after the acquire's answer, so `importing` keeps the mark and the import does not start for an abandoned session. `data-legacy-import.luau` retires the store between the acquire's answer and the import, and while the import runs.
- An import that replaces an existing profile. `Import` only ever merges: its result keeps the session's grants and adds `Legacy`. The code stages the record's purchase ids after the import's record has replaced the payload, so a replacing import would keep the ids and drop their grants, which `GrantWithId` forbids. `docs/profile-stores.md` requires an import over an existing profile to merge into `stored`.
- A third session. With two tokens, a carry can only meet the record its own server's sealed session left, so the epoch check that keeps a carry from replacing another session's later writes is in the model but no trace reaches it. For the same reason an abandoned load's lease removal never meets a lease a third session took.
- The player's data beyond the grants and the import. Saves other than grants are renewals in the model: they change no payload but extend the lease.
- The jitter of renewals. Each tick makes a renewal due for every active session, where the code renews about every `RENEW_INTERVAL_SECONDS`, with jitter.

## Each property catches its bug

`mutants.toml` holds the mutants. Each changes one part of the model, and `cargo xtask tla` fails unless TLC reports a counterexample to the property that mutant names.

| Mutant | Bug | Counterexample |
| --- | --- | --- |
| `no-token-fence` | An owned write lands without checking that its session still holds the lease. | `AckedDurable`, 22 states |
| `local-outlives-lease` | A confirmed write extends local authority past the lease and the takeover after it. | `OneAuthority`, 18 states |
| `authority-from-the-answer` | A load counts its local authority from its acquire's answer, not its send. | `OneAuthority`, 16 states |
| `takeover-before-expiry` | An acquire takes a lease that has not expired. | `OneAuthority`, 12 states |
| `grant-a-staged-id-again` | `ProcessReceipt` grants a purchase whose id the session already staged. | `GrantOnce`, 10 states |
| `grant-without-authority` | `ProcessReceipt` grants into a session that is not writable. | `StopsWriting`, 7 states |
| `grant-without-its-id` | An owned write stores the grant without its purchase id. | `GrantWithId`, 9 states |
| `acknowledged-before-durable` | A callback answers `PurchaseGranted` before the record holds the id. | `AckedDurable`, 8 states |
| `durable-from-the-snapshot` | A confirmed write marks durable the ids staged after its snapshot. | `DurableStored`, 11 states |
| `effects-without-a-grant` | `PurchaseGranted` runs effects this session never staged. | `EffectsOnce`, 18 states |
| `import-ignores-the-mark` | A load runs the legacy import when the record holds the mark. | `ImportOnce`, 14 states |
| `mark-apart-from-the-import` | An owned write stores the imported record without the mark. | `ImportOnce`, 15 states |
| `carry-drops-the-mark` | An acquire that carries a sealed session's imported record leaves its mark behind. | `ImportOnce`, 17 states |
| `import-outlives-authority` | An import that returns after its safe bound still makes its session writable. | `NoWriteAfterLoss`, 11 states |
| `leave-keeps-the-import` | A player who leaves during the import does not end the load. | `NoWriteAfterLoss`, 10 states |
| `writable-before-admission` | An import result that admission refuses still makes its session writable. | `AdmittedBeforeWritable`, 6 states |

`no-token-fence` breaks the data, not authority: a superseded session's write that lands late finds its own safe bound passed, so its confirmation seals it rather than extending it. The fence is what keeps that write from replacing the record of the session that took over.
