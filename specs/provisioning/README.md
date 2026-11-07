# Provisioning model

A TLA+ model of `crucible provision <env>`, the infrastructure-as-code step in `design/architecture.md` ("Infrastructure as code", rules 1 to 7). It covers one environment and a few resource keys. The model includes:

- the config's desired set, with `retired`, `untrack` and `moved_from`;
- the lock on disk;
- the intent journal at `.crucible/journal/<env>.json`;
- Roblox's resources;
- each step of the provisioner.

PR 8's `crucible provision` must conform to it through a `proptest-state-machine` test, so each provisioner action is one operation of the Rust code. The [action list](#actions-the-rust-code-mirrors) gives the mapping.

Run every configuration with `pnpm specs:check`. To run some of them, pass part of a name: `node specs/check.mjs Moves`.

| File | Contents |
| --- | --- |
| `Provisioning.tla` | The model, its invariants and properties |
| `MCProvisioning.tla` | Constants for TLC: the keys, kinds and names |
| `*.cfg` | One TLC configuration each. The first line states the expected result. |

The model matches `design/architecture.md` at commit `3c36049`. No provisioning code exists yet. When PR 8 lands, record here the files the conformance test covers and the commit it last matched.

## What the model says, in English

### Invariants

These hold in every reachable state, with one provisioner.

1. **At most one remote resource exists per key** (`AtMostOneRemotePerKey`). Each Roblox resource carries a ghost tag naming the key that created or adopted it. No key ever tags two resources. `moved_from` passes the tag to the new key. `untrack` clears it, because the key gives the resource up.
2. **The lock never names a resource that doesn't exist** (`LockNamesExisting`). Every lock entry names an existing Roblox resource of that key's kind. Products, passes and badges have separate ID spaces, so a badge ID in a product's entry counts as missing.
3. **No create follows an unknown outcome without an adopt check first** (`NoBlindCreate`). A create's outcome is unsettled from the moment it is sent until the lock records its ID or a response says it did not commit. While it is unsettled, no create runs for that key. Only an adopt check can settle it, and that check must search a listing taken after the create for the exact name the create sent.
4. **A badge create never spends Robux** (`NoPaidBadge`). Roblox charges 100 Robux after the free daily badges. The create sends `expectedCost: 0`, so Roblox refuses instead, and the run reports the badge as deferred.
5. **Removing a key from the config without `retired` or `untrack` is an error, never a silent delete.** Three checks cover it:
   - `RemovedIsError`: no run gets past start while the config drops a key that is still managed in the lock or has an intent.
   - `NoSilentDelete`, an action property: a lock entry leaves only through `untrack` or `moved_from`.
   - `NoSilentRetire`, an action property: Roblox takes a resource off sale only for a key whose config says `retired`.
6. **`moved_from` keeps the ID** (`MovedKeepsId`, an action property). When the old key's entry leaves the lock, the new key's entry has the same ID in the same step.

Supporting invariants:

- `LockInjective`: two keys never hold the same ID.
- `LockOwnsItsResource`: a key's lock entry names the resource tagged with that key.
- `NoCreateBesideMatch`: no create runs while the run's own listing shows an unlocked resource of that kind with that exact name.
- `RunAdvances`: a started run always has a next step, so the Rust loop never spins or hangs.
- `TypeOK`.

### Liveness

`Progress`, checked in `Liveness.cfg`. Assume three things:

- the developer reruns `provision`, which is weak fairness on the provisioner;
- config edits eventually stop;
- crashes, lost responses and rejections eventually stop.

Then every key the config manages gets a lock entry or a reported deferral, unless the final config is invalid. The model runs out of free badges and refills them each day without fairness, so a deferral can repeat forever. That is why a deferral counts as progress.

## Configurations

These were measured on the Windows laptop (16 cores, `-workers auto`, Temurin 21, tla2tools 1.7.4) with `pnpm specs:check`.

| Config | Keys | What it checks | Expected | Distinct states | Time |
| --- | --- | --- | --- | --- | --- |
| `Provisioning.cfg` | product `p`, badge `b` (same name), `p` may already exist by hand | every invariant and action property, unbounded faults and edits, `MaxId = 2` | pass | 1,341,496 | 21 s |
| `Moves.cfg` | `p`, `p2` with `moved_from = "p"`, `p` may exist by hand | the same, `MaxId = 3` | pass | 2,302,088 | 40 s |
| `Liveness.cfg` | `p`, `p2`, `b` | `Progress` with 2 faults and 2 edits, `MaxId = 4` | pass | 154,234 | 23 s |
| `LostLockSameName.cfg` | `p`, `b` | the checkout loses its uncommitted lock and journal, and no edit follows | pass | 3,249 | 2 s |
| `Concurrent.cfg` | `p` | two provisioners, each with its own checkout | violation of `AtMostOneRemotePerKey` | 1,200 to 1,600 | 2 s |
| `ListLag.cfg` | `p` | a listing can miss a resource created just before | violation of `AtMostOneRemotePerKey` | 1,400 to 1,500 | 2 s |
| `LostLock.cfg` | `p` | the checkout loses its lock and journal, then the config renames the resource | violation of `AtMostOneRemotePerKey` | 3,000 to 4,500 | 2 s |

The whole run takes about 92 seconds. For a violation, the count is the states TLC explored before it stopped at the first counterexample. With several workers that number changes from run to run, and so do the state counts in the mutation table. Trace lengths can also differ by a few states.

Keeping the models small:

- The two main configs split the keys. Products with badges is one config, and moves is the other.
- `Provisioning.cfg` with `MaxId = 3` holds every invariant, but explores 15.5 million states in 2 min 19 s.
- IDs come from a counter, as Roblox assigns them, so they need no symmetry reduction.
- Keys differ in kind, name and `moved_from`, so they are not symmetric.
- `Concurrent.cfg` uses symmetry over the two provisioners.
- Passes follow the product path, a list endpoint plus create and update, so they get no key of their own.

### The gaps, as counterexamples

TLC reports each of these as a short trace. They show the model's limits, so they are expected results, not failures.

- **Two provisioners** (`Concurrent.cfg`, documented in rule 6 and in "Pitfalls"). Both list and find nothing, and both create. Their locks are separate files, so neither can see the other. The trace has 11 states.
- **A lagging listing** (`ListLag.cfg`, not in the architecture doc). A create commits and its response is lost. The next run's listing doesn't show the new resource yet. The intent's adopt check finds nothing, clears the intent, and creates again. Rule 3 says the journal "narrows the gap". It narrows it only if the list endpoints show a committed create by the next run. The gated live tests in PR 7 should measure that lag on each of the three list endpoints.
- **Lost local state plus a rename** (`LostLock.cfg`, not in the doc). A run creates a resource. Then the lock change is never committed (`git checkout -- crucible.lock`, a fresh clone, or a deleted `.crucible`). Then the config changes the display name. Adopt-by-name searches the new name and creates a second resource. Without the rename, adopt-by-name recovers everything (`LostLockSameName.cfg` passes).

## Actions the Rust code mirrors

Each provisioner action in `Provisioning.tla` is one operation. A crash can fall between any two of them. "Lock write" and "journal write" each mean writing the whole file to a temp file and renaming it over the old one.

| Action | Rust operation | Effect |
| --- | --- | --- |
| `Start(p)` | Read `crucible.toml`, `crucible.lock` and the journal, then validate them (`Valid`) | Invalid: exit 2 and write nothing. Valid: the run starts. |
| `List(p)` | List developer products, list game passes, and list public badges (`GET badges.roblox.com/v1/universes/{id}/badges`). These are reads. | Stores the listing in memory |
| `ResolveIntent(p, k)` | For an intent an earlier run left, search the listing for the intent's kind and recorded name, skipping IDs the lock already holds | One match: lock write adopting it. None: journal write removing the intent. Two or more: the run stops. |
| `ClearIntent(p, k)` | Journal write removing `k`'s intent | Runs after the lock records the ID, or after a rejected or deferred create |
| `Move(p, k)` | Lock write moving the `moved_from` key's entry to `k` | The ID, fingerprint and status move unchanged |
| `Untrack(p, k)` | Lock write removing `k`'s entry | Roblox is left alone |
| `Update(p, k)` | `PATCH` the product, pass or badge with the config's fields. Retire sets off sale or `enabled: false`. Restore sets the reverse. Safe to repeat. | Success, or a failure that may have applied |
| `WriteLockUpdated(p, k)` | Lock write of `k`'s fingerprint and status | Runs only after a successful update |
| `AdoptCheck(p, k)` | For a managed key with no lock entry, search the listing for the config's kind and name, skipping locked IDs | One match: lock write adopting it, with an unknown fingerprint, so an update follows. None: create. Two or more: the run stops. |
| `WriteIntent(p, k)` | Journal write recording `k`, its kind and the exact name about to be sent | |
| `Create(p, k)` | One create call. It is not idempotent and is never repeated in the run. A badge sends `expectedCost: 0`. | Accepted with an ID, unknown (exit 5), rejected (4xx), or deferred (a refused paid badge) |
| `WriteLockCreated(p, k)` | Lock write of the new ID and the fingerprint sent | Followed by `ClearIntent` |
| `Finish(p)` | Exit once every key is settled. The exit code reports unknown, rejected, deferred and failed keys. | |
| `Crash(p)` | The process dies | Memory is lost. The lock, the journal and Roblox keep every finished write. |

The other actions belong to the environment:

- `Edit`: the developer edits the config between runs.
- `CommitLock` and `Discard`: git, in `LostLock` only.
- `NewDay`: free badges refill.
- `Propagate`: the listing catches up, in `ListLag` only.

A run handles keys in any order the model allows, and the Rust code may fix one order. Before any move, untrack, update or create, every intent left by an earlier run must be resolved or cleared (`NoStaleIntents`).

The conformance test should port `Next` to Rust as the reference. It drives `crucible-iac` against a fake Open Cloud that picks each create's outcome and can kill the client between operations. After each operation it checks that the step is one the model allows from the same state, and that the invariants hold.

## Design decisions the model forced

`design/architecture.md` doesn't state any of these. Each one came from a counterexample or from a mutation below.

1. **The intent records the exact name it sends, and resolution searches that name.** Suppose a run crashes after the create commits, and the config then renames the resource. Searching the config's current name misses the resource and creates a second one (M11).
2. **An adopt check matches kind as well as name.** A "VIP" pass and a "VIP" badge are normal. Matching on name alone puts a badge ID in a product's lock entry (M3).
3. **An adopt check skips every ID the lock already holds, retired entries included.** A retired resource keeps its name, so a later key with that name would adopt it (M10, in `Moves.cfg`).
4. **An adopted entry gets an unknown fingerprint.** The next step updates the resource to match the config, so the plan shows an update after every adoption.
5. **An intent for a key the config dropped is an error, like a dropped managed key.** Its create may have committed, which would leave a live resource that nothing tracks. Rule 5 names only lock entries.
6. **A retired entry may stay in the lock after its key leaves the config.** Removing a retired key from the config is not an error. The entry stays, so its ID stays out of adoption, and declaring the key again puts the same resource back on sale.
7. **Three `moved_from` cases are errors at start:**
   - the old key is still declared;
   - both keys hold lock entries;
   - the new key holds an entry while the old key has an intent.

   The last case has two candidate owners. TLC reached it by renaming a key, renaming it back, and crashing in between.
8. **A move waits until the old key's intent is resolved,** and a move is one lock write.
9. **`untrack` gives the key up.** Declaring the key again later adopts by name or creates a new resource. `AtMostOneRemotePerKey` counts only resources the key still claims.
10. **"Before anything else" means before any write for any key.** Every intent an earlier run left is resolved first, then moves, untracks, updates and creates follow.
11. **An adopt check on an intent that finds nothing clears the intent, and the same run may create the key.** This is safe only if listings show committed creates by the next run (the `ListLag` gap). If live tests show a lag, the run should instead report the key and leave the create to a later run.
12. **After an unknown outcome, the run leaves the intent, makes no further call for that key, continues with the other keys, and exits 5.** Rule 1's "never again" applies to the run. The next run's adopt check decides whether to create.
13. **A rejected or deferred create clears its intent,** because Roblox committed nothing. If the client crashes first, the next run's adopt check finds nothing and clears it.
14. **The lock write comes before the journal clear, and each one is an atomic whole-file write.** The reverse order loses the ID when a crash falls between the two writes (M12).
15. **Two matches stop the whole run before it writes anything else,** and the run asks for `crucible adopt <kind> <key> <id>`. Stopping only that key also passes the safety checks. Stopping the run is the simpler contract.
16. **A resource made by hand is adopted only if the config names it exactly on the first provision.** TLC's first counterexample came from this. A product made by hand, then renamed in the config before its first provision, gets a second product, because adopt-by-name searches the new name. Provision cannot see the link, so `crucible plan` must mark each create as "no existing match" and each adoption as "adopt", so the reviewer catches it. `crucible adopt` handles the case when the name must change.
17. **Losing an uncommitted lock and then renaming creates a duplicate** (`LostLock.cfg`). Rule 6's single-provisioner policy does not cover it. Possible mitigations, which need your decision:
    - `provision` refuses to start while `crucible.lock` has uncommitted changes from an earlier run;
    - `plan` lists every unlocked resource of each declared kind as a possible orphan;
    - a create needs `--allow-create` when such resources exist.
18. **The config is read once at start.** Edits made during a run take effect in the next run.

## Mutations

Each invariant was shown to have teeth. The mutation was applied to a scratch copy of the model, TLC was run, and the counterexample was recorded. None of the mutations remain in the model. "Isolated" means the config checked only the named property, which shows the property catches the bug without help from a sibling invariant.

| # | Mutation | Config | First property TLC reports | Isolated |
| --- | --- | --- | --- | --- |
| M1 | After an unknown outcome, create again in the same run | `Provisioning` | `AtMostOneRemotePerKey` (8 states) | `NoBlindCreate` (8 states) |
| M2 | Ignore the journal at start | `Provisioning` | `RunAdvances` (10 states) | `NoBlindCreate` (13 states) |
| M3 | Adopt by name without checking the kind | `Provisioning` | `LockNamesExisting` (4 states) | `LockOwnsItsResource` (12 states) |
| M4 | Send `expectedCost: 100` on badge creates | `Provisioning` | `NoPaidBadge` (20 states) | |
| M5 | Treat a removed key as untracked: skip validation, then drop its entry | `Provisioning` | `RemovedIsError` (8 states) | `NoSilentDelete` (9 states) |
| M5b | Treat a removed key as retired | `Provisioning` | `RemovedIsError` (7 states) | `NoSilentRetire` (9 states) |
| M6 | Rename by dropping the old entry, so the new key creates | `Moves` | `MovedKeepsId` (14 states) | `AtMostOneRemotePerKey` (13 states) |
| M7 | Skip the adopt check before a create | `Provisioning` | `NoCreateBesideMatch` (6 states) | `AtMostOneRemotePerKey` alone: no violation, 3,661,380 states |
| M7c | The same mutation | `LostLockSameName` | `AtMostOneRemotePerKey` (15 states) | |
| M8 | Create without writing the intent | `Provisioning` | `RunAdvances` (8 states) | `NoBlindCreate` (13 states) |
| M9 | Keep an intent whose adopt check found nothing | `Liveness` | `Progress` (11 states, then the run stalls on the intent forever) | |
| M10 | Adopt a resource another key already holds | `Moves` | `LockInjective` (22 states) | `LockOwnsItsResource` (22 states) |
| M10b | The same mutation | `Provisioning` | no violation, 1,341,496 states | |
| M11 | Resolve an intent by the config's current name | `Provisioning` | `AtMostOneRemotePerKey` (12 states) | `NoBlindCreate` (14 states) |
| M12 | Clear the intent before writing the lock | `Provisioning` | `RunAdvances` (9 states) | `AtMostOneRemotePerKey` (14 states) |
| M13 | Leave a failed update out of the finished states | `Provisioning` | `RunAdvances` (6 states) | |

What the results show:

- **M2, M8 and M12** also leave a run with no next step, so `RunAdvances` reports them first. The isolated runs show the duplicate or the blind create each mutation causes.
- **M7** in `Provisioning.cfg` creates no second resource for any key. With a durable lock and journal, the first-deploy adopt check protects only resources made by hand, and `NoCreateBesideMatch` catches that case. Its other purpose shows in `LostLockSameName.cfg` (M7c): after a lost lock, the adopt check is what prevents a duplicate.
- **M10b** finds nothing with distinct names and no moves. The skip of locked IDs matters once a retired or moved resource keeps a name another key wants, which `Moves.cfg` reaches (M10).

## What CI needs

The `specs:check` script is ready for CI. No CI job runs it yet. A job needs:

- Java 11 or later, for example `actions/setup-java` with Temurin 21. The script looks for `$JAVA`, then `$JAVA_HOME/bin/java`, then `java` on `PATH`.
- Node 24 and `pnpm specs:check`. No `pnpm install` is needed, because the script uses only Node built-ins.
- Network access to GitHub releases on the first run. The script downloads `tla2tools.jar` v1.7.4 into `specs/.tools/` (gitignored) and checks its SHA-256, `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`. A cache keyed on that hash skips the download.
- About 92 seconds on 16 cores. A 4-core runner should take a few minutes, because `Moves.cfg` and `Provisioning.cfg` dominate.

The script exits 1 when any configuration's result differs from its `\* expect:` line, and then prints TLC's full output for that configuration.
