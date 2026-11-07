# Provisioning model

A TLA+ model of `crucible provision <env>`, the infrastructure-as-code step in `design/architecture.md` ("Infrastructure as code", rules 1 to 7). It covers one environment and a few resource keys:

- the config's desired set, with `retired`, `untrack` and `moved_from`;
- the lock and the intent journal on disk;
- Roblox's resources, and creates Roblox may still commit after their response was lost;
- each page of each list endpoint, and each update call;
- the plan review that approves creates;
- the operator's `crucible adopt` and `crucible resolve --not-created`.

PR 8's `crucible provision` must conform to it through a `proptest-state-machine` test, so each provisioner action is one operation of the Rust code. The [action list](#actions-the-rust-code-mirrors) gives the mapping.

Run every configuration with `pnpm specs:check`. To run some of them, pass part of a name, as in `node specs/check.mjs Moves`. A name that matches nothing fails. Run the mutation campaign with `pnpm specs:mutate`, or pass mutation IDs, as in `node specs/mutate.mjs provisioning M9 M15`.

| File | Contents |
| --- | --- |
| `Provisioning.tla` | The model, its invariants and properties |
| `MCProvisioning.tla` | Constants for TLC: the keys, kinds, names and ID limits |
| `*.cfg` | One TLC configuration each. The first line states the expected result. |
| `mutations.json` | The mutation campaign. `specs/mutate.mjs` applies each edit to a copy of the model and checks the result. |

The model matches `design/architecture.md` at commit `3c36049`, plus the decisions below. No provisioning code exists yet. When PR 8 lands, record here the files the conformance test covers and the commit it last matched.

## What the model guarantees

### Invariants

These hold in every reachable state with one provisioner, a durable lock and journal, and the [assumptions](#assumptions) below.

1. **At most one remote resource exists per key** (`AtMostOneRemotePerKey`). Each Roblox resource carries a ghost tag naming the key that created or adopted it. No key ever tags two resources. `moved_from` passes the tag to the new key.
2. **The journal or the lock accounts for every create that may have committed** (`JournalCoversCreates`). While a create is in flight at Roblox, or has committed without a lock entry, its key keeps its intent. This is what stops a later run from creating again.
3. **No create runs while an earlier create of the same lineage may commit or has committed** (`NoBlindCreate`). The lineage of a `moved_from` key is the key and its old key.
4. **The lock never names a resource that doesn't exist** (`LockNamesExisting`). An entry names a resource of its key's kind. Each kind has its own ID space, so product 42 and badge 42 are two resources.
5. **A badge create never spends Robux** (`NoPaidBadge`). The create sends `expectedCost: 0`, so Roblox refuses a paid create, and the run reports the badge as deferred.
6. **Removing a key from the config without `retired` or `untrack` is an error, never a silent delete.** Three checks cover it:
   - `RemovedIsError`: no run gets past start while the config drops a key that is still managed in the lock or has an intent.
   - `NoSilentDelete`, an action property: a lock entry leaves the lock only through `moved_from`.
   - `NoSilentRetire`, an action property: Roblox takes a resource off sale only for a key whose config says `retired`.
7. **`moved_from` keeps the ID** (`MovedKeepsId`, an action property). When the old key's entry leaves the lock, the new key's entry has the same ID in the same step.

Supporting invariants:

- `LockInjective`: two keys of one kind never hold the same ID.
- `LockOwnsItsResource`: a key's lock entry names the resource tagged with that key.
- `NoCreateBesideMatch`: no create runs while the run's own listing shows an unlocked resource of that kind with that exact name.
- `RunAdvances`: a started run always has a next step, so the Rust loop never spins or hangs.
- `TypeOK`.

### Convergence

`Converges`, checked in `Liveness.cfg` and `LivenessLag.cfg`. Assume these things:

- the developer reruns `provision`, which is weak fairness on the provisioner;
- the reviewer eventually approves the plan, and the operator eventually runs `crucible resolve --not-created` for a create Roblox dropped;
- a day passes, listings catch up, and Roblox eventually finishes or drops each create whose response was lost;
- config edits eventually stop, and so do failed calls, lost responses and crashes.

Then each key ends in its desired state, unless the final config is invalid:

- a managed or retired key has a lock entry whose fingerprint, icon hash and status match the config;
- Roblox shows that resource with the config's name and icon, on sale when managed and off sale when retired;
- an untracked key has no entry or an `untracked` entry;
- no intent remains.

That covers retiring and restoring. A mutation that never updates, never retires, never restores or skips the badge icon fails it.

## Assumptions

The guarantees above hold only under these contracts. The fake Open Cloud in PR 8's conformance test must offer each failure mode listed here, and must not offer more than the model does.

- **One provisioner per environment.** This is rule 6. `Concurrent.cfg` shows the duplicate two provisioners make.
- **A rejected create proves Roblox committed nothing.** Only a validation 4xx (exit 4), a 429 or a request that never left the process (exit 7), and the refused paid badge count. Every other failure is unknown: a 5xx, a timeout, a lost response, or a malformed 2xx. The fake service must return each of these.
- **A create whose response was lost may still commit later**, at any time, or never. The model holds it in `pending` until `Settle` commits or drops it. The fake service must be able to commit such a request after later listings have run.
- **Listings are paged, with no snapshot across pages.** Each page reads the current state of its slice. A resource that existed before the listing began appears on exactly one page, because IDs never change and nothing is deleted. A resource that commits during a listing may be missed. The fake service must page in a stable order over IDs.
- **A listing may lag.** With `ListLag`, a new resource stays invisible to every list endpoint until it propagates. That covers the public badge list, whose freshness Roblox doesn't guarantee. The fake service must be able to hide a new resource from its lists.
- **An update's calls are separate.** A product or pass update is one call that carries the icon. A badge update is a metadata `PATCH` and, if the icon changed, an icon upload. Any call may fail after Roblox applied it.
- **The reviewer reads the plan.** A careful reviewer declines a create listed beside an unlocked resource that is the key's own lost resource. `LostLockApproved.cfg` shows the duplicate when the reviewer approves anyway.
- **The operator is truthful.** `crucible resolve --not-created` is safe only when the create never committed and never will. `crucible adopt` is safe when it names the key's own resource, or any unlocked resource while no create of the key can have committed. Mutation M21 shows the duplicate a false `--not-created` makes.
- **The journal survives until its intents resolve.** Deleting `.crucible` while it holds an intent loses the only record of that create (`LostJournal.cfg`).
- **Nothing else changes Roblox.** The model has no dashboard edits and no external removals. A field edited in Creations is overwritten at the next update and is not seen before it.
- **A lost update response doesn't land after a later update.** The model lets a failed call apply at once or never, not later. A late landing would leave a field the lock records as applied. Detecting it needs a read-back that compares the listing's fields with the config.

## Configurations

These were measured on the Windows laptop (16 cores, `-workers auto`, Temurin 21.0.13, tla2tools 1.7.4) with `pnpm specs:check`.

| Config | Keys | What it checks | Expected | Distinct states | Time |
| --- | --- | --- | --- | --- | --- |
| `Provisioning.cfg` | product `p`, badge `b` (same name), `p` may exist by hand | every safety property, unbounded faults and edits, two products and one badge | pass | 1,906,048 | 40 s |
| `Moves.cfg` | `p`, `p2` with `moved_from = "p"`, `p` may exist by hand | every safety property, unbounded faults and edits, three products | pass | 295,172 | 10 s |
| `Collision.cfg` | `p` and `b`, product 1 and badge 1 both exist by hand | every safety property with one edit | pass | 29,848 | 4 s |
| `ListLag.cfg` | `b`, which may exist by hand | every safety property while listings lag, unbounded faults and edits | pass | 16,514 | 3 s |
| `LostLock.cfg` | `p` | the checkout loses its uncommitted lock, with a careful reviewer | pass | 15,896 | 3 s |
| `Liveness.cfg` | `p`, `p2`, `b` | `Converges` with 2 faults and 2 edits | pass | 307,602 | 43 s |
| `LivenessLag.cfg` | `b` | `Converges` while listings lag, with 3 faults and 3 edits | pass | 63,674 | 10 s |
| `Concurrent.cfg` | `p` | two provisioners, each with its own checkout | violation of `AtMostOneRemotePerKey` | 3,177 | 2 s |
| `LostLockApproved.cfg` | `p` | `LostLock.cfg` with a reviewer who approves anyway | violation of `AtMostOneRemotePerKey` | 4,735 | 2 s |
| `LostJournal.cfg` | `p` | the checkout loses its journal while it holds an intent | violation of `AtMostOneRemotePerKey` | 1,584 | 2 s |

The whole run takes 120 seconds.

For a violation, the count is the states TLC explored before it stopped at the first counterexample. With several workers that number changes from run to run, and so do the counts in the mutation table.

Keeping the models small:

- Each kind's ID limit (`MaxId`) is the smallest that still lets a duplicate exist. Two products and one badge are enough in `Provisioning.cfg`, because the badge's paid-create path starts from a day with no free badges left.
- `Provisioning.cfg` and `Moves.cfg` allow unbounded edits and faults. `Collision.cfg` allows one edit, because its point is the first adopt check. The two liveness configs bound both, because convergence needs them to stop.
- Lagging listings get their own config with one badge key. Lag works the same for every kind, and the public badge list is the endpoint whose freshness Roblox doesn't promise. With lag on all three keys of `Liveness.cfg`, TLC passes in 2 min 26 s, which is too slow for every run.
- Endpoints are listed in a fixed order, products and then badges. The Rust code must use the same order.
- Passes follow the product path, a list endpoint plus create and update, so they get no key of their own.
- `Concurrent.cfg` uses symmetry over the two provisioners.

### Why two pages are enough

`PageCount = 2` in every configuration. A page read can matter only by what it misses. With IDs that never change and no deletes, a page misses a resource only if the resource committed after that page was read. Two pages already give every such resource both outcomes: it lands before its page is read and is seen, or after, and is missed. A third page adds one more boundary of the same kind. Lag also hides any new resource from any page, so the configurations with lag already cover every subset of new resources a listing of any length can miss. Products and badges then add a boundary between endpoints, which is the same case again.

### The gaps, as counterexamples

TLC reports each of these as a short trace. They are the model's documented residual risks, so they are expected results.

- **Two provisioners** (`Concurrent.cfg`, rule 6 and "Pitfalls"). Both list, both find nothing, and both create. Their locks are separate files, so neither can see the other.
- **A reviewer who approves a create beside the key's own orphan** (`LostLockApproved.cfg`). The checkout loses an uncommitted lock, and the config renames the resource. Adopt-by-name searches the new name and finds nothing. The plan lists the old resource as a possible orphan, and only the reviewer can tell it is the same product. `LostLock.cfg` is the same world with a careful reviewer, and it passes.
- **A lost journal** (`LostJournal.cfg`). A create's response is lost, and then `.crucible` is deleted. Roblox commits the create after the next listing, and the next run creates again. Nothing durable remembers the first create.

## Actions the Rust code mirrors

Each provisioner action in `Provisioning.tla` is one operation. A crash can fall between any two. "Lock write" and "journal write" each mean the durable whole-file write in decision 14.

| Action | Rust operation | Effect |
| --- | --- | --- |
| `Start(p)` | Read `crucible.toml`, `crucible.lock` and the journal, then validate them (`Valid`) | Invalid: exit 2 and write nothing. Valid: the listing starts. |
| `ListPage(p, x)` | One `GET` of one page: developer products, game passes, or the public badge list (`GET badges.roblox.com/v1/universes/{id}/badges`). Products and passes come before badges. | Success adds the page to the listing. Failure stops the run before any write. |
| `Approve(p, all)` | Show the plan built from the listing and take the review: a TTY confirmation, or a `--plan` file that matches (decision 17). `all = FALSE` is a run with no approval. | Records the approved creates |
| `ResolveIntent(p, k)` | For an intent an earlier run left, search the listing for its kind and recorded name, skipping identities the lock holds | One match: lock write adopting it. None: report `unresolved` and keep the intent. Two or more: stop the run. |
| `ClearIntent(p, k)` | Journal write removing `k`'s intent | Runs after the lock records the ID, or after a create that proves no commit |
| `Move(p, k)` | Lock write moving the `moved_from` key's entry to `k` | The ID, fingerprint, icon hash and status move unchanged |
| `Untrack(p, k)` | Lock write marking `k`'s entry `untracked` | Roblox is left alone |
| `WriteLockStale(p, k)` | Lock write clearing `k`'s fingerprint, and its icon hash if an icon upload follows | Runs before every update |
| `Update(p, k)` | `PATCH` the product or pass with every field and the icon, or `PATCH` the badge's name, description and `enabled`. Retire sets off sale or `enabled: false`, and restore sets the reverse. | Success, or a failure that may have applied |
| `UpdateIcon(p, k)` | `POST /legacy-publish/v1/badges/{badgeId}/icon`, only when the icon hash differs | Success, or a failure that may have applied |
| `WriteLockUpdated(p, k)` | Lock write of `k`'s fingerprint, icon hash and status | Runs only after every call the update needs succeeded |
| `AdoptCheck(p, k)` | For a managed key with no lock entry, search the listing for the config's kind and name, skipping identities the lock holds | One match: lock write adopting it, with an unknown fingerprint, so an update follows. None: create if approved, otherwise report `unapproved`. Two or more: stop the run. |
| `WriteIntent(p, k)` | Journal write recording `k`, its kind and the exact name about to be sent | |
| `Create(p, k)` | One create call. It is never repeated in the run. A badge sends `expectedCost: 0`. | Accepted with an ID, unknown, rejected, or deferred |
| `WriteLockCreated(p, k)` | Lock write of the new ID, fingerprint and icon hash | Followed by `ClearIntent` |
| `Finish(p)` | Exit once every key is settled | Exit 0 when `Unfinished(p)` is empty. Otherwise non-zero, and 5 when any key is `unknown` or `unresolved`. |
| `Crash(p)` | The process dies | Memory is lost. The lock, the journal and Roblox keep every finished write. |

Each key's state in the run is `run[p].ks[k].s`. The conformance test compares it after each step. `todo` means done once the key needs nothing. `unknown`, `unresolved`, `unapproved`, `rejected`, `deferred` and `failed` are the results the run reports.

The operator's commands are separate processes, run between provisions:

- `OperatorAdopt(p, k, i)` is `crucible adopt <kind> <key> <id>`. It reads the resource by ID, writes the lock entry, then clears any intent. A crash between those two writes leaves an entry with an intent, which the next run's `ClearIntent` handles.
- `OperatorNotCreated(p, k)` is `crucible resolve <env> <key> --not-created`, one journal write.

The environment's actions:

- `Edit`: the developer edits the config between runs.
- `CommitLock` and `DiscardLock`: git commits the lock, or the checkout loses its uncommitted lock. `DiscardJournal`: `.crucible` is deleted.
- `NewDay`: free badges refill.
- `Propagate`: a lagging listing catches up.
- `Settle`: Roblox commits or drops a create whose response was lost.

A run handles keys in any order the model allows, and the Rust code may fix one order. Before any move, untrack, update or create, every intent an earlier run left must be resolved, cleared, or reported `unresolved` (`NoStaleIntents`).

The conformance test should port `Next` to Rust as the reference. It drives `crucible-iac` against a fake Open Cloud that picks each call's outcome, can commit a lost create later, can hide new resources from its lists, and can kill the client between operations. After each operation it checks that the step is one the model allows from the same state, and that the invariants hold.

## Design decisions

`design/architecture.md` doesn't state these. Each came from a counterexample or a mutation, named in brackets.

1. **The intent records the exact name it sends, and resolution searches that name.** Searching the config's current name after a rename can adopt a different resource while the create commits later (M11).
2. **An adopt check matches kind as well as name.** A "VIP" pass and a "VIP" badge are normal (M3).
3. **An adopt check skips every (kind, id) identity the lock holds, retired and untracked entries included.** Numeric IDs overlap across kinds, so locking product 1 must not hide badge 1 (M15, `Collision.cfg`). A retired resource keeps its name, so a later key with that name would otherwise adopt it (M10).
4. **An adopted entry gets an unknown fingerprint.** An update follows, so the plan shows an update after every adoption.
5. **An intent for a key the config dropped is an error, like a dropped managed key.** Its create may have committed. Rule 5 names only lock entries.
6. **A retired or untracked entry may stay in the lock after its key leaves the config.** Its ID stays out of adoption, and declaring the key again brings back the same resource.
7. **Three `moved_from` cases are errors at start:**
   - the old key is still declared;
   - both keys hold lock entries;
   - one key holds an entry while the other holds an intent, in either direction.

   The last case has two candidate owners. The previous model checked only one direction, and TLC reached the other once an empty listing stopped clearing intents (M28).
8. **A move waits until the old key's intent is resolved, and a new key creates nothing while its old key holds an intent.** Without the second rule, the old key's create can commit beside the new key's (M27).
9. **`untrack` keeps the entry, marked `untracked`.** Roblox is left alone and the key gets no further updates. The adopt check skips its ID, and declaring the key again restores the same resource. The previous model removed the entry. TLC then found duplicates in two ways. A lagging listing hid the untracked resource. A lost lock let a rename adopt the untracked resource while the key's newer one was orphaned (M23).
10. **"Before anything else" means before any write for any key.** Every intent an earlier run left is resolved or reported first.
11. **An empty listing never settles an unknown create.** The intent stays, and the key reports `unresolved` with a non-zero exit, until one of two things happens:
    - a later listing shows the exact sent name, and the run adopts it;
    - the operator runs `crucible adopt <kind> <key> <id>`, or `crucible resolve <env> <key> --not-created`, which is safe only if the create never committed.

    Clearing the intent on an empty listing duplicates the resource when the create commits late or the listing lags (M9).
12. **After an unknown outcome, the run leaves the intent and makes no further call for that key.** It continues with the other keys and exits 5. Rule 1's "never again" applies to the run.
13. **Only an outcome that proves no commit clears an intent.** That is a validation 4xx, a 429, a request that never left the process, or a refused paid badge. Everything else is unknown.
14. **The lock write comes before the journal clear, and each write is durable before the next call.** Each write goes to a temp file, is flushed with `fsync` (`FlushFileBuffers` on Windows), is renamed over the old file, and then the directory is flushed. On Windows the rename uses `MoveFileEx` with `MOVEFILE_WRITE_THROUGH`. A rename alone survives a process crash, because the OS keeps its page cache. Only a flushed write survives a power loss. The model's `Crash` is a process crash. It also covers a power loss only if the intent is flushed before the create is sent and the lock is flushed before the intent is cleared.
15. **Two matches stop the run before its next write, and ask for `crucible adopt <kind> <key> <id>`.** Writes the run already made stay. They are resolved intents, moves, untracks and updates for other keys, and each one is consistent on its own.
16. **A resource made by hand is adopted automatically only when the config names it exactly.** Otherwise the plan lists the create as "no exact-name match observed", beside every unlocked resource of that kind as a possible orphan. `crucible adopt` handles a resource whose name must change, and adopts in bulk for FD's first run.
17. **A create needs approval from one reviewed plan.** On a TTY, `provision` prints the plan and asks. Otherwise it takes `--plan <file>` from `crucible plan --out`. The file is bound to the environment, the hash of `crucible.toml` and `crucible.lock`, the (kind, id) identities its listing saw, and the exact (kind, key, name) creates. A stale plan is refused, and there is no global bypass. The model computes the approval from the run's own listing in `Approve`. A fresh plan file approves the same creates, because it saw the same config, lock and identities. The names of unlocked resources only change through a lock write or a new identity. Binding the plan to the lock and the identities, not only to the inputs, closes a hole. A plan made before a create would otherwise still be fresh after the lock that recorded the create is lost.
18. **The config, lock and journal are read once at start.** The run writes the lock and the journal as whole files from memory. Nothing else may write them during a run, because the run would overwrite that write. An edit to `crucible.toml` during a run takes effect at the next start, and it makes any plan stale.
19. **Before an update, the lock forgets what the update will change.** `WriteLockStale` clears the fingerprint, and the icon hash when an upload follows. After every call succeeds, the lock records the new fingerprint, icon hash and status. A crash or failure in between leaves an update for the next run, even after the config is reverted. TLC found the drift without it: a retire failed after Roblox applied it, the config was then restored, and the badge stayed disabled (M24).
20. **A badge update is a metadata `PATCH`, then an icon upload only when the icon hash differs.** The fingerprint advances only after both succeed (M19, M20).
21. **The fingerprint covers every applied input, the name included.** After `moved_from`, the new key's name differs from the old key's, and a version-only fingerprint never sent the rename (found by `Converges`).
22. **Every list page succeeds before the run writes anything.** A failed page stops the run. Counting it as empty hides an orphan from the review (M22).

## Mutations

Each property was shown to have teeth. `specs/mutate.mjs` applies each entry of `mutations.json` to a copy of the model, runs TLC on the named configuration, and checks that the first violation is the expected one. "Only" means the configuration checked only that property, which shows the property catches the bug without help from a sibling. The script fails if any property of a passing configuration has no mutation that catches it.

| # | Mutation | Config | First property TLC reports |
| --- | --- | --- | --- |
| M1 | After an unknown outcome, create again in the same run | `Provisioning` | `NoBlindCreate` |
| M1i | The same | `Provisioning`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M2 | Ignore the journal at start | `LivenessLag` | `Converges` |
| M3 | Adopt by name without checking the kind | `Provisioning` | `LockNamesExisting` |
| M4 | Send `expectedCost: 100` on badge creates | `Provisioning` | `NoPaidBadge` |
| M5 | Skip validation at start | `Provisioning` | `RemovedIsError` |
| M5b | Skip validation and treat a removed key as retired | `Provisioning`, only `NoSilentRetire` | `NoSilentRetire` |
| M6 | Rename by dropping the old entry, so the new key creates | `Moves` | `JournalCoversCreates` |
| M6i | The same | `Moves`, only `MovedKeepsId` | `MovedKeepsId` |
| M7 | Skip the adopt check before a create | `LivenessLag` | `Converges` |
| M8 | Create without writing the intent | `Provisioning` | `JournalCoversCreates` |
| M9 | An empty listing settles an unknown create, as before this round | `Provisioning` | `JournalCoversCreates` |
| M9i | The same | `ListLag`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M10 | Adopt a resource another key already holds | `Moves` | `LockInjective` |
| M10i | The same | `Moves`, only `LockOwnsItsResource` | `LockOwnsItsResource` |
| M11 | Resolve an intent by the config's current name | `Provisioning` | `AtMostOneRemotePerKey` |
| M12 | Clear the intent before writing the lock | `Provisioning` | `JournalCoversCreates` |
| M13 | Leave a failed update out of the finished states | `Provisioning` | `RunAdvances` |
| M14 | Never update | `LivenessLag` | `Converges` |
| M14p | The same | `Liveness` | `Converges` |
| M15 | Skip locked IDs of every kind, not (kind, id) identities | `Collision` | `NoCreateBesideMatch` |
| M16 | The reviewer approves a create beside the key's own orphan | `LostLock` | `AtMostOneRemotePerKey` |
| M17 | Retiring leaves the resource on sale | `LivenessLag` | `Converges` |
| M18 | Restoring leaves the resource off sale | `LivenessLag` | `Converges` |
| M19 | Skip the badge icon upload | `LivenessLag` | `Converges` |
| M20 | Count a failed icon upload as done | `LivenessLag` | `Converges` |
| M21 | `--not-created` while the create may still commit | `Provisioning` | `JournalCoversCreates` |
| M21i | The same | `Provisioning`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M22 | A failed list page counts as an empty page | `LostLock` | `AtMostOneRemotePerKey` |
| M23 | Untrack drops the entry and gives the resource up, as before this round | `Provisioning` | `NoSilentDelete` |
| M23l | The same | `ListLag`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M23k | The same | `LostLock` | `AtMostOneRemotePerKey` |
| M24 | Update without first marking the lock entry stale | `LivenessLag` | `Converges` |
| M25 | Never adopt the create a later listing shows | `LivenessLag` | `Converges` |
| M26 | A run enters a phase the model doesn't name | `Provisioning` | `TypeOK` |
| M27 | Create a moved key while its old key holds an intent | `Moves` | `NoBlindCreate` |
| M28 | Accept an old key's entry beside the new key's intent | `Moves` | `RemovedIsError` |

TLC prints "Temporal properties were violated" for a liveness failure without naming the property. Both liveness configurations check only `Converges`.

What the results show:

- **M9 and M23** are the two rules this round replaced. Each one fails a safety property again, so the new rules carry weight.
- **M7** no longer breaks safety. The plan approves no create that an exact match covers, so skipping the adopt check only leaves the key unapproved forever, which `Converges` catches.
- **M2** also breaks only convergence, for the same reason: the plan never approves a key that holds an intent.
- **M6** fails `JournalCoversCreates` first, because the moved claim has no entry and no intent. With that check removed, the careful reviewer declines the new key's create beside its own orphan, so `MovedKeepsId` is what catches it.

## What CI needs

The `specs:check` script is ready for CI. No CI job runs it yet. A job needs:

- Java 11 or later, for example `actions/setup-java` with Temurin 21. The scripts look for `$JAVA`, then `$JAVA_HOME/bin/java`, then `java` on `PATH`.
- Node 24 and `pnpm specs:check`. No `pnpm install` is needed, because the scripts use only Node built-ins.
- Network access to GitHub releases on the first run. The script downloads `tla2tools.jar` v1.7.4 into `specs/.tools/` (gitignored) and checks its SHA-256, `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`. A cache keyed on that hash skips the download.
- About 2 minutes on 16 cores. A 4-core runner should take a few minutes, because `Liveness.cfg` and `Provisioning.cfg` dominate.

The script exits 1 when any configuration's result differs from its `\* expect:` line, and then prints TLC's full output for that configuration. `pnpm specs:mutate` takes about 2 minutes more and can run in the same job.
