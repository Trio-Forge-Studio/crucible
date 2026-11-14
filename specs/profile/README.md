# Profile model

`Profile.tla` models one player's profile under a profile store: the lease each server's session takes in the DataStore record, the local authority a session holds while its safe bound lasts, the owned writes that renew the lease and carry the session's data, the release that ends it, and Roblox's `ProcessReceipt` callbacks that grant developer products into it. `packages/crucible/src/internal/data/profiles.ts`, `session.ts`, `lease.ts` and `receipts.ts` implement it.

Run it with `cargo xtask tla`. The command needs Java 11 or later on `PATH` or in `JAVA_HOME`. It downloads TLC 1.7.4 into `target/tla` and checks the jar's SHA-256, or uses the jar `TLA2TOOLS_JAR` names.

## States

A session is `sess[t]`, one per token. Its `phase` is the `Authority` union in `session.ts`:

- `loading` until its acquire's answer activates it or ends the load. A load the player leaves ends at once and owes the write that removes its own lease, if its acquire took one.
- `active` while it holds the lease. It is writable exactly while `now` is before its safe bound, which an activation sets and each confirmed write extends, from the earliest send of the request that may have stored it plus `LOCAL_AUTHORITY_SECONDS`.
- `sealed`, with the reason, once the player leaves, its safe bound passes, or a write finds the lease gone. Writes stop at once, and the session sends its release.
- `ended`, with the release's outcome: `committed`, `fenced` or `given up`.

A purchase in a session is `Receipt` in `session.ts`: staged with its effects when the grant ran, durable once a confirmed write's record holds its id, and durable without effects once `PurchaseGranted` ran them. In the model, a session's `staged`, `durable` and `effects` sets hold the same facts.

The record, `store`, holds the lease's token, epoch and expiry, the commit marker of the last owned write (its token, revision and op), the payload, and the receipts. An acquire takes the lease when it is absent, expired past `TAKEOVER_GRACE_SECONDS`, or held by one of this server's own sealed sessions whose release settled. Then it carries that session's snapshot when the record does not already hold it. An owned write lands only for the token and epoch that hold the lease, before its expiry, and a retry whose op already landed reads `already`.

## What TLC checks

| Property | In plain English | TLA+ |
| --- | --- | --- |
| `OneAuthority` | At most one session is writable at any moment, across every server. | `Cardinality({t : Writable(t)}) <= 1` |
| `GrantOnce` | No purchase is granted twice, in the record or in any session's data. | `store.payload[p] <= 1` and `sess[t].value[p] <= 1` |
| `GrantWithId` | A grant and its purchase id are stored in one revision: the record holds one exactly when it holds the other. | `(p \in store.receipts) <=> (store.payload[p] >= 1)` |
| `AckedDurable` | Roblox is answered `PurchaseGranted` only for a purchase whose grant the record holds, and the record keeps holding it. | `\A p \in acked : store.payload[p] = 1 /\ p \in store.receipts` |
| `DurableStored` | What an active or sealed session believes durable, the record holds. | `sess[t].durable \subseteq store.receipts` |
| `EffectsOnce` | `afterGrant` effects run at most once per purchase, and only once Roblox was told it is granted. | `ran[p] <= 1 /\ (ran[p] = 1 => p \in acked)` |
| `StopsWriting` | A session's data changes only while it is writable: a sealed, ended or expired session never writes. | `[][sess'[t].value # sess[t].value => Writable(t)]_vars` |

`ran` is a ghost variable: the code keeps no count of the effects it ran, and `EffectsOnce` counts with it.

`Profile.cfg` checks two servers, one purchase, two sessions, one lost answer, a lease of 3, local authority of 2 and a grace of 0, with the clock stopping at 4. The player starts on one server and may teleport to the other once. TLC 1.7.4 explores 11,671,109 states, 3,089,025 of them distinct, to depth 45, in about two minutes on two workers.

## Assumptions

- **A1.** Every server reads one clock. The code's `TAKEOVER_GRACE_SECONDS` and the margin between `LEASE_SECONDS` and `LOCAL_AUTHORITY_SECONDS` absorb skew between servers' clocks. The model sets the grace to 0 and keeps the margin at one tick, so it checks the protocol with none of the skew allowance left over.
- **A2.** A server sends one request for the player's key at a time, which the scheduler's per-key lane gives. A load that the player leaves stops waiting, and its acquire may still land.
- **A3.** A request lands in the DataStore atomically, at some time after it was sent. Its answer can be lost after it landed or before, and the code learns nothing from a lost answer but that it may have landed.
- **A4.** Roblox calls `ProcessReceipt` for a purchase it has not been told is granted, on the server the player is on, and again later until one call answers `PurchaseGranted`.

## What the model leaves out

- Handoff between servers, Subsystem retirement, shutdown and the inbox. Each seals a session for a reason the model's `Leave` already covers, and `data-handoff.luau`, `data-shutdown.luau` and the inbox specs cover their own rules.
- A third session. With two tokens, a carry can only meet the record its own server's sealed session left, so the epoch check that keeps a carry from replacing another session's later writes is in the model but no trace reaches it. For the same reason an abandoned load's lease removal never meets a lease a third session took.
- The player's data beyond the grants, migrations and the schema. Saves other than grants are renewals in the model: they change no payload but extend the lease.
- The jitter of renewals. Each tick makes a renewal due for every active session, where the code renews about every `RENEW_INTERVAL_SECONDS`, with jitter.

## Each property catches its bug

`mutants.toml` holds the mutants. Each changes one part of the model, and `cargo xtask tla` fails unless TLC reports a counterexample to the property that mutant names.

| Mutant | Bug | Counterexample |
| --- | --- | --- |
| `no-token-fence` | An owned write lands without checking that its session still holds the lease. | `AckedDurable`, 20 states |
| `local-outlives-lease` | A confirmed write extends local authority past the lease and the takeover after it. | `OneAuthority`, 18 states |
| `authority-from-the-answer` | A load counts its local authority from its acquire's answer, not its send. | `OneAuthority`, 14 states |
| `takeover-before-expiry` | An acquire takes a lease that has not expired. | `OneAuthority`, 10 states |
| `grant-a-staged-id-again` | `ProcessReceipt` grants a purchase whose id the session already staged. | `GrantOnce`, 9 states |
| `grant-without-authority` | `ProcessReceipt` grants into a session that is not writable. | `StopsWriting`, 8 states |
| `grant-without-its-id` | An owned write stores the grant without its purchase id. | `GrantWithId`, 8 states |
| `acknowledged-before-durable` | A callback answers `PurchaseGranted` before the record holds the id. | `AckedDurable`, 8 states |
| `durable-from-the-snapshot` | A confirmed write marks durable the ids staged after its snapshot. | `DurableStored`, 10 states |
| `effects-without-a-grant` | `PurchaseGranted` runs effects this session never staged. | `EffectsOnce`, 18 states |

`no-token-fence` breaks the data, not authority: a superseded session's write that lands late finds its own safe bound passed, so its confirmation seals it rather than extending it. The fence is what keeps that write from replacing the record of the session that took over.
