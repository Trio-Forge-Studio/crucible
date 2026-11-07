undefined24823 | 4 s |undefined117502 | 7 s |undefined572 | 2 s |undefined91022 | 5 s |undefined1076372 | 30 s |undefined456980 | 9 s |undefined270 | 4 s |undefined104102 | 7 s |undefined4264 | 3 s |undefined4141 | 2 s |undefined259066 | 8 s |undefined1658 | 2 s |undefined1354 | 2 s |undefined220854 | 35 s |undefined220854 | 33 s |undefined31160 | 6 s |undefined129526 | 8 s |undefined3313 | 2 s |undefined329092 | 15 s |# Provisioning models

Two TLA+ models of `crucible provision <env>`, the infrastructure-as-code step in `design/architecture.md` ("Infrastructure as code", rules 1 to 7). PR 8's `crucible provision` must conform to both through a `proptest-state-machine` test against a fake Open Cloud. Each provisioner action is one operation of the Rust code, and the [action lists](#actions-the-rust-code-mirrors) give the mapping.

- `provisioning/` models resource identity. It covers which Roblox resource each key's lock entry names, the intent journal, creates whose response was lost, the plan and plan files, and the operator's commands.
- `reconcile/` models how a locked resource's fields converge. It covers the listed fields the run reads back, the icon it can't read back, metadata updates that land late, and edits in Creations.

[How the two models compose](#how-the-two-models-compose) explains why checking them apart is enough.

Run every configuration with `pnpm specs:check`. To run some of them, pass part of a name, as in `node specs/check.mjs Moves`. A name that matches nothing fails. Run both mutation campaigns with `pnpm specs:mutate`, or pass a model and mutation IDs, as in `node specs/mutate.mjs provisioning M11 M31`.

| File | Contents |
| --- | --- |
| `provisioning/Provisioning.tla` | The identity model, its invariants and properties |
| `provisioning/MCProvisioning.tla` | Constants for TLC: the keys, kinds, names and ID limits |
| `reconcile/Reconcile.tla` | The field model, its invariants and properties |
| `reconcile/MCReconcile.tla` | The TLC entry module |
| `*/*.cfg` | One TLC configuration each. The first line states the expected result. |
| `*/mutations.json` | Each model's mutation campaign |
| `check.mjs`, `mutate.mjs`, `tlc.mjs` | The runners. `mutate.mjs` applies each mutation to a copy of the model and checks the result. |

The models match `design/architecture.md` at commit `3c36049`, plus the [design decisions](#design-decisions) below. No provisioning code exists yet. When PR 8 lands, record here the files the conformance test covers and the commit it last matched.

## What the models guarantee

### Identity invariants

`Provisioning.tla` checks these in every reachable state, with one provisioner per environment, a journal that survives, and the [assumptions](#assumptions) below.

1. **At most one remote resource exists per key** (`AtMostOneRemotePerKey`). Each Roblox resource has a ghost owner, the key that created or adopted it. No key owns two. `moved_from` passes ownership to the new key.
2. **The journal or the lock accounts for every create that may have committed** (`JournalCoversCreates`). While a create is in flight at Roblox, or has committed without a lock entry, its key keeps its intent.
3. **No create runs while an earlier create of the same lineage may commit or has committed** (`NoBlindCreate`). The lineage of a `moved_from` key is the key and its old key.
4. **No create runs beside a match in its own listing** (`NoCreateBesideMatch`). A match is an unlocked resource that carries a create marker of the key's lineage, or an unlocked, unmarked resource of the key's kind with its exact name.
5. **The lock names only resources that exist and that its key owns** (`LockNamesExisting`, `LockOwnsItsResource`), and never gives one ID of a kind to two keys (`LockInjective`). Each kind has its own ID space, so product 42 and badge 42 are two resources.
6. **A `retired` lock entry names a resource that is off sale or disabled** (`RetiredIsOff`). That is what makes removing a retired key from the config safe.
7. **A badge create never spends Robux** (`NoPaidBadge`). The create sends `expectedCost: 0`, so Roblox refuses a paid create, and the run reports the badge as deferred.
8. **Removing a key from the config without `retired` or `untrack` is an error, never a silent delete.** Three checks cover it.
   - `RemovedIsError`: no run lists or writes while the config drops a key that is still managed in the lock or has an intent.
   - `NoSilentDelete`, an action property: a lock entry leaves the lock only through `moved_from`.
   - `NoSilentRetire`, an action property: Roblox takes a resource off sale only through an update sent while its key's config, or its retired entry, said retired. A late update counts by what its config said when it was sent.
9. **`moved_from` keeps the ID** (`MovedKeepsId`, an action property).
10. **`provision --plan` accepts only a plan whose environment, config, lock, listed identities and creates all match the run's own** (`PlanIsFresh`, an action property).
11. **Each step writes at most one of the lock, the journal, the plan file and Roblox** (`OneWritePerStep`, an action property). So a crash can fall between any two writes, and the conformance test can kill the client at each of them.
12. **A started run always has a next step** (`RunAdvances`), so the Rust loop never spins or hangs. `TypeOK` holds too.

### Field invariants

`Reconcile.tla` checks these.

- **The lock's icon hash never names an icon Roblox doesn't show** (`IconHashTrue`). The hash is either unknown or true.
- `OneWritePerStep`, for the lock and Roblox, and `TypeOK`.

### Convergence

Assume the developer reruns `provision`, the reviewer approves its plan, an operator command runs to its end, a day passes, listings catch up, and Roblox settles each request whose response was lost. Assume also that config edits, edits in Creations, failed calls, lost responses and crashes eventually stop. Three properties follow, and they keep automatic reconciliation apart from manual recovery.

- **Fields** (`Converges` in `Reconcile.tla`). The resource ends with every listed field the config gives, the config's icon, and that icon's hash in the lock. The listed fields are the name, the description, the price, and availability, which can be off for a managed badge. `Reconcile.cfg` and `ReconcileLag.cfg` check it, with metadata updates that land after newer ones and edits in Creations.
- **Identity without the operator** (`ConvergesOrAsks`, `LivenessAuto.cfg`). Each key converges, or ends with an intent whose create Roblox dropped or never received. A crash between the journal write and the create call leaves the same intent. Only `crucible resolve <env> <key> --not-created` settles it, because no listing can tell a dropped create from a late one.
- **Identity with the operator** (`Converges` in `Provisioning.tla`, `Liveness.cfg` and `LivenessLag.cfg`). If the operator also runs `--not-created` for each such intent, every key converges. `NoOperator.cfg` is the counterexample without the operator: Roblox drops a create, and its intent stays forever.

A converged key has no intent, and a managed key has a lock entry. A key with an entry that the config declares `managed` or `retired` shows the config's name, no marker, and the config's availability, and the entry's status matches the config. A retired key that was never created has no entry, because there is nothing to retire. An absent key's retired entry stays off. An untracked key has no entry or an `untracked` one.

## Assumptions

The guarantees hold only under these contracts. The fake Open Cloud in PR 8's conformance test must offer each failure mode listed here, and must not offer more than the models do.

- **One provisioner per environment.** This is rule 6. `Concurrent.cfg` shows the duplicate two provisioners make.
- **A rejected create proves Roblox committed nothing.** Only a validation 4xx (exit 4), a 429 or a request that never left the process (exit 7), and the refused paid badge count. Every other failure is unknown: a 5xx, a timeout, a lost response, or a malformed 2xx. The fake service must return each of these.
- **A create whose response was lost may commit later**, at any time, or never. The fake service must be able to commit such a request after later listings have run.
- **A create marker is unforgeable.** A create writes `[crucible:<env>/<key>/<nonce>]` as the description, with a fresh random nonce that the intent records. No hand-made resource carries a marker, and only an update by `provision` removes one. Each list endpoint returns the description. PR 8 must confirm that for developer products, game passes and the public badge list.
- **Listings are paged, with no snapshot across pages.** Each page reads the current state of its slice. A resource that existed before the listing began appears on exactly one page, because IDs never change and nothing is deleted. A resource that commits during a listing may be missed. The fake service must page in a stable order over IDs.
- **A listing may lag.** With `ListLag`, a new resource stays hidden from every list endpoint until it propagates (`Provisioning.tla`), and a list can return a field value Roblox has since replaced (`Reconcile.tla`). Resources made by hand may start hidden too. That covers the public badge list, whose freshness Roblox doesn't guarantee. The fake service must be able to do both.
- **An update is two calls.** A metadata `PATCH` carries every listed field and no icon. An icon upload follows only when the icon hash differs. For a product or pass, it is a `PATCH` with only the image. For a badge, it is `POST /legacy-publish/v1/badges/{badgeId}/icon`. Any call may fail after Roblox applied it, and a lost metadata `PATCH` may apply after a newer one. The fake service must be able to apply a lost `PATCH` at any later time.
- **A lost icon upload doesn't land after a newer one.** The icon can't be read back, so nothing can detect it. `LateIcon.cfg` shows the stale icon it leaves.
- **The reviewer reads the plan.** A careful reviewer declines a plan that shows a create beside the key's own unlocked resource. `LostLockApproved.cfg` shows the duplicate when the reviewer approves anyway.
- **The operator is truthful.** `crucible resolve --not-created` is safe only when the create never committed and never will. `crucible adopt` is safe when it names the lineage's own resource, or any unlocked resource while no create of the lineage can have committed. Mutation M21 shows the duplicate a false `--not-created` makes.
- **Edits in Creations reach only listed fields.** The run reads listed fields back, so an edit in Creations to a name, description, price or availability is overwritten at the next run. An icon changed in Creations is not detected, for the same reason as a late icon upload.
- **The config changes only between runs.** The models let no edit happen during a run. The Rust code reads `crucible.toml`, the lock and the journal once at start, and the conformance test must not edit them mid-run.
- **Two declared keys of one kind never share a display name.** The parser rejects that config. The models state it as an `ASSUME` on the first config and as a precondition of `Edit`. Keys of different kinds may share a name, as a "VIP" pass and a "VIP" badge do.

### The local loss the product tolerates

The journal lives at `~/.crucible/journal/<universeId>/<env>.json`, outside the checkout. Losing the checkout keeps it. That covers `git clean`, `git checkout -- crucible.lock`, a fresh clone, and a second checkout on the same machine. `LostLock.cfg` passes with a lost lock and a careful reviewer.

Two losses remain residual risks:

- **Losing the home folder while an intent's create is in flight**, or moving to another machine, when the create's commit is still hidden as the next create is approved (`LostJournal.cfg`). If Roblox has committed the create and the list shows it, its marker adopts it, and `LostJournalSeen.cfg` passes even with a reviewer who approves every plan.
- **Losing an uncommitted lock while the list still hides the key's resource** (`LostLockLag.cfg`). Neither a marker nor the review can see a resource the list doesn't return.

## How the two models compose

The Rust code is one program. Projected onto `Provisioning.tla`'s variables, each of its traces must be a trace of `Provisioning.tla`, with the icon steps as stuttering steps. Projected onto one resource's fields, it must be a trace of `Reconcile.tla`, with the identity steps as stuttering steps or config edits. The conformance test checks both projections after each operation. The two models share only the lock entry and the listed fields of a locked resource, and these points make checking them apart sound.

1. **`Provisioning.tla`'s `Update` is `Reconcile.tla`'s `UpdateMeta`, seen through what identity reads.** It keeps the name, whether the description still carries a marker, and availability. The icon calls and the icon hash belong to `Reconcile.tla` alone, and no identity step reads them. `Update` may also run with no drift that identity can see, which stands for a price or description edit.
2. **Identity never reads a locked resource's fields.** Adoption by marker or by name, intent resolution, and the orphan review all skip every identity the lock holds. While the lock survives, a resource stays locked once a key adopts or creates it (`NoSilentDelete`). So the late metadata updates and the edits in Creations that `Reconcile.tla` adds cannot change an identity decision. `Provisioning.tla` turns on `LateUpdates` where the lock can be lost (`LostLock.cfg`). It also turns it on in `LivenessLag.cfg` and `LivenessAuto.cfg`, where a late restore can land after a retire.
3. **`Reconcile.tla` starts from every state identity can leave a locked resource in.** That is any field values, a marker in the description (a description that differs from the config's), and an icon hash that is unknown or true. Adoption leaves the hash unknown, and a move carries the old key's hash. A move changes which config applies to the resource, which `Reconcile.tla` covers as config edits.
4. **Resources don't share field state.** A run reads each resource's fields from its own list entry, and each `PATCH` and upload names one resource ID. So one resource is enough. It is a product, because a badge has the same calls and fields with no price.
5. **The conclusions chain.** Identity convergence ends with every managed key locked to its resource and no intent left. Field convergence then brings every listed field and the icon to the config.

## Configurations

These were measured on the Windows laptop (16 cores, `-workers auto`, Temurin 21.0.13, tla2tools 1.7.4) with `pnpm specs:check`.

| Config | Keys | What it checks | Expected | Distinct states | Time |
| --- | --- | --- | --- | --- | --- |
| `provisioning/Collision.cfg` | p, b | Two products and a badge may exist by hand, under any names, with one edit. Product 1 and badge 1 are two resources. | pass | 329092 | 17 s |
| `provisioning/Concurrent.cfg` | p | The documented gap: two provisioners, each with its own checkout, mint two resources for one key. | violation of `AtMostOneRemotePerKey` | 3857 | 3 s |
| `provisioning/ListLag.cfg` | b | Two badges may exist by hand, hidden from the list until they propagate, and a create whose response was lost can commit later. | pass | 129526 | 7 s |
| `provisioning/Liveness.cfg` | p, p2 | Convergence with manual recovery, once edits and faults stop: every key ends locked to its resource with the config's name, no marker, and the config's availability, and no intent remains. | pass | 31160 | 8 s |
| `provisioning/LivenessAuto.cfg` | b | Automatic reconciliation: with no operator, a badge converges, or ends with an intent that only `crucible resolve --not-created` can settle. | pass | 220854 | 28 s |
| `provisioning/LivenessLag.cfg` | b | Convergence with manual recovery of a badge while the list lags and a lost update can land late. | pass | 220854 | 47 s |
| `provisioning/LostJournal.cfg` | p | The residual risk of a lost home folder: the journal is lost while it holds an intent, and Roblox commits the create after the next listing. | violation of `AtMostOneRemotePerKey` | 1536 | 3 s |
| `provisioning/LostJournalSeen.cfg` | p | The home folder loses the journal while it holds an intent, and Roblox settles the create before the next listing. The marker identifies it, so even a reviewer who approves every plan makes no duplicate. | pass | 1658 | 4 s |
| `provisioning/LostLock.cfg` | p | The checkout can lose its uncommitted lock, a lost update can land late, and the config can rename the product. The reviewer declines a plan with a create beside the key's own orphan, and the operator adopts the orphan. | pass | 259066 | 10 s |
| `provisioning/LostLockApproved.cfg` | p | The residual risk of review: after a lost lock and a rename, the reviewer approves a plan with a create beside the key's own orphan. | violation of `AtMostOneRemotePerKey` | 4089 | 2 s |
| `provisioning/LostLockLag.cfg` | p | The residual risk of a lost lock while the list still hides the key's resource: neither its marker nor the review can see it. | violation of `AtMostOneRemotePerKey` | 4418 | 3 s |
| `provisioning/Moves.cfg` | p, p2 | `p2` declares `moved_from = "p"`, and `p` may already exist by hand. Every safety property must hold. | pass | 104102 | 7 s |
| `provisioning/NoOperator.cfg` | b | The residual dependence on the operator: Roblox drops a create whose response was lost, and nobody runs `crucible resolve --not-created`. | violation of `Converges` | 270 | 2 s |
| `provisioning/PlanFiles.cfg` | p | `crucible plan --out` for either environment, then `provision --plan`, while the list lags and a hand-made product may exist. Every accepted plan matches its environment, config, lock, identities and creates. | pass | 456980 | 14 s |
| `provisioning/Provisioning.cfg` | p, b | One provisioner, a product and a badge with the same display name, and a product that may already exist by hand. Every safety property must hold. | pass | 1076372 | 35 s |
| `provisioning/SameKind.cfg` | p, q | Two product keys that can trade names between runs, while a create of one may still commit under the name the other now uses. | pass | 91022 | 8 s |
| `reconcile/LateIcon.cfg` | one product | The residual risk of the icon: an upload whose response was lost lands after a newer one, and no read-back can see it. | violation of `IconHashTrue` | 572 | 2 s |
| `reconcile/Reconcile.cfg` | one product | A product's name, description, price and availability vary independently, a lost metadata PATCH can land after a newer one, and Creations can edit any listed field. The fields and the icon converge once that stops. | pass | 117502 | 8 s |
| `reconcile/ReconcileLag.cfg` | one product | The list can return a field value Roblox has since replaced, as the public badge list does. Convergence still holds once the list catches up. | pass | 24823 | 3 s |

The whole run takes 183 seconds.

For a violation, the count is the states TLC explored before it stopped at the first counterexample. With several workers that number changes from run to run, and so do the counts in the mutation table.

### Why the constants are enough

- **Two pages.** A page read can matter only by what it misses. With IDs that never change and no deletes, a page misses a resource only if the resource committed after that page was read. Two pages already give every such resource both outcomes. Lag hides any new resource from any page, so the configurations with lag cover every subset a listing of any length can miss.
- **ID limits.** Each kind's `MaxId` is the smallest that still lets a duplicate exist beside the resources made by hand. `Collision.cfg` holds two products and a badge made by hand, and `ListLag.cfg` two hidden badges, so several hand-made resources of one kind start visible or hidden.
- **Two names.** A run only compares names for equality. Two names give a match and a miss, let two product keys trade names (`SameKind.cfg`), and let a product and a badge share one (`Provisioning.cfg`).
- **Two nonces.** A fresh nonce must differ from every nonce a create of its key may still carry. With one provisioner, at most one such create exists when a new one is sent, because the earlier one has an intent, a lock entry, or a visible marker that blocks the create. So two values are enough. Symmetry over nonces halves the safety runs.
- **Two values per field and two icons.** A run only compares a field with the config's. Two values give both outcomes, and a late update carries a value that differs from the current one. `Reconcile.cfg` varies four fields independently. `ReconcileLag.cfg` uses two, because lag works the same for each field.
- **Bounds.** `Provisioning.cfg`, `Moves.cfg`, `ListLag.cfg`, `LostLock.cfg` and `SameKind.cfg` allow unbounded faults, and all but `SameKind.cfg` unbounded edits. `Collision.cfg` allows one edit, because its point is the first adopt check. Liveness needs faults and edits to stop, so those configurations bound both. `Liveness.cfg` keeps only the `moved_from` lineage, because the badge path has its own liveness configurations with lag.
- **Fixed order.** Endpoints are listed in a fixed order, products and then badges, and the Rust code must use the same order. Passes follow the product path, a list endpoint plus create and update, so they get no key of their own.

### The gaps, as counterexamples

TLC reports each of these as a short trace. They are the documented residual risks, so they are expected results.

- **Two provisioners** (`Concurrent.cfg`, rule 6). Both list, both find nothing, and both create. Their locks are separate files, so neither can see the other.
- **A reviewer who approves a create beside the key's own orphan** (`LostLockApproved.cfg`). The checkout loses an uncommitted lock, the update already removed the marker, and the config renames the resource. Adopt-by-name searches the new name and finds nothing. The plan lists the old resource as a possible orphan, and only the reviewer can tell it is the same product.
- **A lost lock while the list hides the resource** (`LostLockLag.cfg`). The run sees no marker and no name, and the plan shows no orphan.
- **A lost home folder** (`LostJournal.cfg`). A create's response is lost, and then the journal is deleted. Roblox commits the create after the next listing, and the next run creates again.
- **No operator** (`NoOperator.cfg`). Roblox drops a create whose response was lost. Nothing but `--not-created` can settle its intent.
- **A late icon upload** (`LateIcon.cfg`). An upload whose response was lost lands after a newer one. The lock records the newer hash, and no read-back can see the old icon.

## Actions the Rust code mirrors

Each provisioner action is one operation. A crash can fall between any two. "Lock write" and "journal write" each mean the durable whole-file write in decision 16.

### Identity (`Provisioning.tla`)

| Action | Rust operation | Effect |
| --- | --- | --- |
| `Start(p, mode)` | Read `crucible.toml`, `crucible.lock`, the journal and, for `--plan`, the plan file, then validate them (`Valid`). `mode` is `tty`, `file` for `provision --plan`, or `plan` for `crucible plan --out`. | Invalid: exit 2 and write nothing. Valid: the listing starts. |
| `ListPage(p, x)` | One `GET` of one page: developer products, game passes, or the public badge list (`GET badges.roblox.com/v1/universes/{id}/badges`). Products and passes come before badges. | Success adds the page's kind, ID, name, description and availability to the listing. Failure stops the run before any write. |
| `Approve(p, yes)` | Print the plan this listing gives and take the TTY answer. `yes = FALSE` is a run without approval, such as CI without `--plan`. | Records the plan's creates as approved |
| `UsePlan(p)` | Compare the plan file's environment, config hash, lock hash, listed (kind, id) identities and (kind, key, name) creates with this run's (`PlanMatches`) | All match: approves the plan's creates. Any differs: exit non-zero before any write. |
| `WritePlan(p, e)` | `crucible plan --out` writes the plan file for environment `e` | |
| `ResolveIntent(p, k)` | For an intent an earlier run left, search the unlocked listed resources for a marker of `k`'s lineage | The intent's exact marker: lock write adopting it. A lineage marker with another nonce: stop the run. None: report `unresolved` and keep the intent. |
| `ClearIntent(p, k)` | Journal write removing `k`'s intent | Runs after the lock records the ID, or after a create that proves no commit |
| `Move(p, k)` | Lock write moving the `moved_from` key's entry to `k` | The ID and status move unchanged |
| `Untrack(p, k)` | Lock write marking `k`'s entry `untracked` | Roblox is left alone |
| `WriteLockManaged(p, k)` | Lock write of status `managed` | Before a restore, and when an untracked key is declared again |
| `Update(p, k)` | One metadata `PATCH` with the config's name, real description and availability, sent when the listing shows any drift or doesn't show the resource. An absent key's retired entry gets availability only. | Success, or a failure that applied now, later, or never |
| `WriteLockRetired(p, k)` | Lock write of status `retired` | After a retire succeeded, or when the listing shows the resource off |
| `AdoptCheck(p, k)` | For a managed key with no entry and no intent, search the unlocked listed resources for a marker of `k`'s lineage, then for an unmarked one with `k`'s kind and exact name | One: lock write adopting it. None: create if approved, otherwise report `unapproved`. Two or more: stop the run. |
| `WriteIntent(p, k)` | Journal write recording `k`, its kind, the exact name and a fresh nonce | |
| `Create(p, k)` | One create call with the marker as the description. It is never repeated in the run. A badge sends `expectedCost: 0`. | Accepted with an ID, unknown, rejected, or deferred |
| `WriteLockCreated(p, k)` | Lock write of the new ID with status `managed` | `ClearIntent` and an `Update` follow |
| `Finish(p)` | Exit once every key is settled | Exit 0 when `Unfinished(p)` is empty. Otherwise non-zero, and 5 when any key is `unknown` or `unresolved`. |
| `Crash(p)` | The process dies | Memory is lost. The files and Roblox keep every finished write. |

Each key's state in the run is `run[p].ks[k].s`, and the conformance test compares it after each step. `todo` and `synced` mean done once the key needs nothing. `unknown`, `unresolved`, `unapproved`, `rejected`, `deferred` and `failed` are the results the run reports. Before any move, untrack, lock status write, update or create, every intent an earlier run left must be resolved, cleared, or reported `unresolved` (`NoStaleIntents`). A run handles keys in any order the model allows, and the Rust code may fix one order.

The operator's commands are separate processes, run between provisions:

- `crucible adopt <kind> <key> <id>` is three steps. `AdoptRead(p, k, i)` reads the resource by ID, `AdoptLock(p)` writes the lock entry, and `AdoptClear(p)` clears any intent. A crash can fall between any two, and the next run's `ClearIntent` handles an entry left beside an intent.
- `crucible resolve <env> <key> --not-created` is `OperatorNotCreated(p, k)`, one journal write.

The environment's actions:

- `Edit` is the developer editing the config between runs.
- `CommitLock` and `DiscardCheckout` are git committing the lock and the checkout losing its uncommitted lock. `DiscardHome` is the home folder losing the journal.
- `NewDay` refills free badges, and `Propagate` is a hidden resource reaching the list.
- `Settle` and `SettleUpdate` are Roblox finishing or dropping a create or an update whose response was lost.

### Fields (`Reconcile.tla`)

| Action | Rust operation | Effect |
| --- | --- | --- |
| `Start` | Read the files | The listing starts |
| `List` | The list `GET` that returns this resource | Success records its listed fields. Failure stops the run. |
| `SkipMeta` | Every listed field equals the config's | No call |
| `UpdateMeta` | One metadata `PATCH` with every listed field and no icon | Success, or a failure that applied now, later, or never |
| `WriteIconStale` | Lock write clearing the icon hash | Before an upload, when the hash differs from the config's icon |
| `UploadIcon` | The image-only `PATCH` of a product or pass, or the badge icon `POST` | Success, or a failure that may have applied |
| `WriteIconHash` | Lock write of the new icon hash | Only after the upload succeeded |
| `Finish` | Exit once the hash matches | |
| `Crash` | The process dies | |

The environment's actions are `EditField` and `EditIcon` for the config, `EditInCreations` for a listed field, `LandLate` and `LandLateIcon` for Roblox, and `Propagate` for the list catching up.

## Design decisions

`design/architecture.md` doesn't state these. Each came from a counterexample or a mutation, named in brackets, or from the review of the previous round.

1. **A create carries a marker, and only that marker settles its intent.** The create writes `[crucible:<env>/<key>/<nonce>]` as the description. The intent records the kind, the exact name and the nonce. A listing that shows the exact marker adopts the resource. An exact name without the marker never settles an intent, because a hand-made resource may have that name while the create commits later (M11, the previous round's rule).
2. **An adopt check matches kind as well as name** (M3). A "VIP" pass and a "VIP" badge are normal.
3. **An adopt check skips every (kind, id) identity the lock holds, retired and untracked entries included** (M10, M15). Numeric IDs overlap across kinds, so locking product 1 must not hide badge 1.
4. **A marker of the key's lineage wins over a name, and a name adopts only an unmarked resource.** A run that lost its journal adopts a listed resource that carries the key's marker instead of creating (M30). Another key's marked resource is never adopted by name, even after the keys trade names (M29). A `moved_from` key adopts its old key's marked resource.
5. **A lineage marker with another nonce, during an unresolved intent, stops the run.** It means state was lost and a duplicate already exists. This and the nonce itself are diagnostics. With the stop removed, and separately with any lineage marker accepted as the intent's own, every safety configuration still passes. The duplicate was decided when the second create was sent, so the nonce only lets the error name it. The campaign has no mutation for either, because no property catches one.
6. **Fields come from the listing, and an update follows every create and every marker adoption.** The run compares each listed field with the config and sends a metadata `PATCH` on drift, or when the listing didn't show the resource, as after a create. That `PATCH` writes the real description, which removes the marker (M40, M41).
7. **An intent for a key the config dropped is an error, like a dropped managed key.** Its create may have committed. Rule 5 names only lock entries.
8. **A retired or untracked entry may stay in the lock after its key leaves the config.** Its ID stays out of adoption, and declaring the key again brings back the same resource. A retired entry stays reconciled off, so a late restore that lands after the key left the config is undone.
9. **Three `moved_from` cases are errors at start** (M28). The old key is still declared, both keys hold lock entries, or one key holds an entry while the other holds an intent, in either direction.
10. **A move waits until the old key's intent is resolved, and a new key creates nothing while its old key holds an intent** (M27).
11. **`untrack` keeps the entry, marked `untracked`** (M23). Roblox is left alone, the key gets no further updates, and the adopt check skips its ID. Declaring the key again writes the entry back to `managed` before anything else.
12. **"Before anything else" means before any write for any key.** Every intent an earlier run left is resolved or reported first.
13. **No listing settles an intent by itself.** The intent stays, and the key reports `unresolved` with a non-zero exit, until one of these happens (M9, M25):
    - a later listing shows the intent's exact marker, and the run adopts it;
    - the operator runs `crucible adopt <kind> <key> <id>`;
    - the operator runs `crucible resolve <env> <key> --not-created`, which is safe only if the create never committed and never will.
14. **After an unknown outcome, the run leaves the intent and makes no further call for that key** (M1). It continues with the other keys and exits 5. Rule 1's "never again" applies to the run.
15. **Only an outcome that proves no commit clears an intent.** That is a validation 4xx, a 429, a request that never left the process, or a refused paid badge. Everything else is unknown.
16. **The lock write comes before the journal clear, and each write is durable before the next call** (M12, M32). Each write goes to a temp file, is flushed with `fsync` (`FlushFileBuffers` on Windows), is renamed over the old file, and then the directory is flushed. On Windows the rename uses `MoveFileEx` with `MOVEFILE_WRITE_THROUGH`. A rename alone survives a process crash, because the OS keeps its page cache. Only a flushed write survives a power loss. `OneWritePerStep` checks that no step bundles two writes.
17. **Two matches stop the run before its next write, and ask for `crucible adopt <kind> <key> <id>`.** Writes the run already made stay. They are resolved intents, moves, untracks, lock status writes and updates for other keys, and each one is consistent on its own.
18. **A resource made by hand is adopted automatically only when the config names it exactly, the key has no intent, and no marker of its lineage is listed.** Otherwise the plan lists the create beside every unlocked resource of that kind as a possible orphan. `crucible adopt` handles a resource whose name must change, and adopts in bulk for FD's first run.
19. **A create needs approval from one reviewed plan, which is bound to its inputs and observations.** On a TTY, `provision` prints the plan and asks. Otherwise it takes `--plan <file>` from `crucible plan --out`. The file records the environment, the hash of `crucible.toml`, the hash of `crucible.lock`, the (kind, id) identities its listing saw, and the exact (kind, key, name) creates. `provision --plan` lists again and compares each binding with its own. If any differs, it exits non-zero before any write (M33 to M37). The TTY flow takes the same path with a plan built from the run's own listing, so its bindings match by construction. The reviewer approves the whole plan or none of it. There is no global bypass.
20. **A plan file is not consumed.** A plan whose five bindings all match the run's own is the plan a fresh `crucible plan` would print. So a replay is either rejected or harmless, and `PlanFiles.cfg` passes with every plan reusable any number of times.
21. **The config, lock and journal are read once at start.** The run writes the lock and the journal as whole files from memory. Nothing else may write them during a run, because the run would overwrite that write. The models exclude a config edit during a run. They don't model a snapshot of one.
22. **Listed fields reconcile from the listing, not from a lock fingerprint.** The lock keeps no fingerprint of the name, description, price or availability. A lost `PATCH` that lands after a newer one, or an edit in Creations, shows up as drift at the next run, which repairs it (R4). Each listed field takes part in the comparison on its own (R1, R2), and a managed badge may stay disabled (R3).
23. **The icon goes in its own call for every kind, and the lock keeps its hash.** A product's metadata `PATCH` could carry the image, but a late one would then restore an old icon that nothing can detect (R5). Before an upload, the lock clears the icon hash. After the upload succeeds, it writes the new one (R6, R7, R8). A late icon upload is the residual risk in `LateIcon.cfg`.
24. **The lock's status follows Roblox conservatively.** `managed` is written before a restore or a re-track, and `retired` only after a retire succeeded or the listing shows the resource off (M38, M39). A key whose entry says `retired` can leave the config, so the entry must never say `retired` while the resource may be on sale.
25. **Every list page succeeds before the run writes anything** (M22). A failed page stops the run. Counting it as empty hides an orphan from the review.
26. **The journal lives in the home folder, at `~/.crucible/journal/<universeId>/<env>.json`** (M31). A journal inside the checkout is lost with it, and with it the only record of a create in flight.
27. **`crucible adopt` reads, writes the lock, and clears the intent in three steps** (M32). Each is one write, like every provisioner action.

## Mutations

Each property was shown to have teeth. `specs/mutate.mjs` applies each entry of a model's `mutations.json` to a copy of the model, runs TLC on the named configuration, and checks that the first violation is the expected one. "Only" means the configuration checked only that property, which shows the property catches the bug without help from a sibling. The script fails if any property of a passing configuration has no mutation that catches it, if an edit doesn't match the model exactly once, or if a filter matches no mutation.

| # | Mutation | Config | First property TLC reports |
| --- | --- | --- | --- |
| M1 | After an unknown outcome, create again in the same run | `provisioning/Provisioning` | `NoBlindCreate` |
| M1i | The same | `provisioning/Provisioning`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M2 | Ignore the journal at start | `provisioning/LivenessLag` | `Converges` |
| M3 | Adopt by name without checking the kind | `provisioning/Provisioning` | `LockNamesExisting` |
| M4 | Send `expectedCost: 100` on badge creates | `provisioning/Provisioning` | `NoPaidBadge` |
| M5 | Skip validation at start | `provisioning/Provisioning` | `RemovedIsError` |
| M5b | Skip validation and reconcile a removed key off sale | `provisioning/Provisioning`, only `NoSilentRetire` | `NoSilentRetire` |
| M6 | Rename by dropping the old entry, so the new key creates | `provisioning/Moves` | `JournalCoversCreates` |
| M6i | The same | `provisioning/Moves`, only `MovedKeepsId` | `MovedKeepsId` |
| M7 | Skip the adopt check before a create | `provisioning/LivenessLag` | `Converges` |
| M8 | Create without writing the intent | `provisioning/Provisioning` | `JournalCoversCreates` |
| M9 | An empty listing settles an unknown create | `provisioning/Provisioning` | `JournalCoversCreates` |
| M9i | The same | `provisioning/ListLag`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M10 | Adopt a resource another key already holds | `provisioning/SameKind` | `LockInjective` |
| M10i | The same | `provisioning/SameKind`, only `LockOwnsItsResource` | `LockOwnsItsResource` |
| M11 | Settle an unknown create by its exact name, without the marker (round 2's rule) | `provisioning/ListLag`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M12 | Clear the intent before writing the lock | `provisioning/Provisioning` | `JournalCoversCreates` |
| M13 | Leave a failed update out of the finished states | `provisioning/Provisioning` | `RunAdvances` |
| M14 | Never update | `provisioning/Liveness` | `Converges` |
| M14l | The same | `provisioning/LivenessLag` | `Converges` |
| M15 | Skip locked IDs of every kind, not (kind, id) identities | `provisioning/Collision` | `NoCreateBesideMatch` |
| M16 | The reviewer approves a create beside the key's own orphan | `provisioning/LostLock` | `AtMostOneRemotePerKey` |
| M17 | Retiring leaves the resource on sale | `provisioning/LivenessLag` | `Converges` |
| M18 | Restoring leaves the resource off sale | `provisioning/LivenessLag` | `Converges` |
| M21 | `--not-created` while the create may still commit | `provisioning/Provisioning` | `JournalCoversCreates` |
| M21i | The same | `provisioning/Provisioning`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M22 | A failed list page counts as an empty page | `provisioning/LostLock` | `AtMostOneRemotePerKey` |
| M23 | Untrack drops the entry and gives the resource up | `provisioning/Provisioning` | `NoSilentDelete` |
| M23k | The same | `provisioning/LostLock` | `AtMostOneRemotePerKey` |
| M25 | Never adopt the create a later listing shows | `provisioning/LivenessLag` | `Converges` |
| M25a | The same, without the operator | `provisioning/LivenessAuto` | `ConvergesOrAsks` |
| M26 | A run enters a phase the model doesn't name | `provisioning/Provisioning` | `TypeOK` |
| M27 | Create a moved key while its old key holds an intent | `provisioning/Moves`, only `NoBlindCreate` | `NoBlindCreate` |
| M28 | Accept an old key's entry beside the new key's intent | `provisioning/Moves` | `RemovedIsError` |
| M29 | Adopt an unmarked or marked resource by name alike | `provisioning/SameKind`, only `LockOwnsItsResource` | `LockOwnsItsResource` |
| M30 | Ignore create markers outside an intent, so a lost journal falls back to the name | `provisioning/LostJournalSeen` | `AtMostOneRemotePerKey` |
| M31 | Keep the journal inside the checkout | `provisioning/LostLock`, only `AtMostOneRemotePerKey` | `AtMostOneRemotePerKey` |
| M32 | `crucible adopt` writes the lock and clears the intent in one step | `provisioning/Provisioning` | `OneWritePerStep` |
| M33 | Accept a plan for another environment | `provisioning/PlanFiles` | `PlanIsFresh` |
| M34 | Accept a plan made before a config edit | `provisioning/PlanFiles` | `PlanIsFresh` |
| M35 | Accept a plan made before a lock write | `provisioning/PlanFiles` | `PlanIsFresh` |
| M36 | Accept a plan that saw other identities | `provisioning/PlanFiles` | `PlanIsFresh` |
| M37 | Accept a plan that lists other creates | `provisioning/PlanFiles` | `PlanIsFresh` |
| M38 | Restore a retired resource before the lock says managed | `provisioning/Provisioning` | `RetiredIsOff` |
| M39 | Mark the lock retired before the retire succeeded | `provisioning/Provisioning` | `RetiredIsOff` |
| M40 | The update keeps the create marker in the description | `provisioning/Liveness` | `Converges` |
| M41 | Compare only availability, not the name and the marker | `provisioning/Liveness` | `Converges` |
| R1 | Leave price out of the read-back comparison | `reconcile/Reconcile` | `Converges` |
| R2 | Leave the description out of the read-back comparison | `reconcile/Reconcile` | `Converges` |
| R3 | A managed resource is always sent as available, so a disabled badge cannot stay disabled | `reconcile/Reconcile` | `Converges` |
| R4 | Trust a current icon hash and skip the read-back | `reconcile/Reconcile` | `Converges` |
| R5 | Send the icon in the metadata PATCH, as one product call | `reconcile/Reconcile` | `IconHashTrue` |
| R6 | Upload the icon without first marking the hash stale | `reconcile/Reconcile` | `IconHashTrue` |
| R7 | Write the icon hash after a failed upload | `reconcile/Reconcile` | `IconHashTrue` |
| R8 | Write the icon hash in the upload step | `reconcile/Reconcile` | `OneWritePerStep` |
| R9 | Never upload the icon | `reconcile/Reconcile` | `Converges` |
| R10 | A run enters a phase the model doesn't name | `reconcile/Reconcile` | `TypeOK` |

TLC prints "Temporal properties were violated" for a liveness failure without naming the property. Each liveness configuration checks one temporal property.

What the results show:

- **M11** is the previous round's resolution by exact name. It fails `AtMostOneRemotePerKey` in `ListLag.cfg`, through a trace of the shape the previous review found. A hand-made badge with the desired name starts hidden, so the plan approves a create. The run creates and crashes before its lock write. The next listing shows the hand-made badge but not the new one, and the name match adopts the hand-made badge while the new one keeps the key's claim.
- **M29, M30 and M31** are the new identity rules. Each one fails a safety property again, so each rule carries weight. M30 needs a reviewer who approves every plan, because a careful reviewer also sees the committed create as the key's own orphan. The marker makes the lost-journal case safe without the review.
- **M27** also fails `RemovedIsError`, because the moved key's create leaves an entry beside its old key's intent. The run checks only `NoBlindCreate` to show that property catches the bug alone.
- **M33 to M37** drop one plan binding each. Each one lets `provision --plan` accept a plan that differs from its own in exactly that binding.
- **R4** is the previous round's fingerprint, which trusted the lock instead of the listing. It fails `Converges` because a late `PATCH` or an edit in Creations is never repaired.
- **R5** is the single product call that carries the icon. A late one restores an old icon behind a true-looking hash.

## What CI needs

The `specs:check` script is ready for CI. No CI job runs it yet. A job needs:

- Java 11 or later, for example `actions/setup-java` with Temurin 21. The scripts look for `$JAVA`, then `$JAVA_HOME/bin/java`, then `java` on `PATH`.
- Node 24 and `pnpm specs:check`. No `pnpm install` is needed, because the scripts use only Node built-ins.
- Network access to GitHub releases on the first run. The script downloads `tla2tools.jar` v1.7.4 into `specs/.tools/` (gitignored) and checks its SHA-256, `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`. A cache keyed on that hash skips the download.
- About 3 minutes on 16 cores. A 4-core runner should take a few times longer.

The script exits 1 when any configuration's result differs from its `\* expect:` line, and then prints TLC's full output for that configuration. `pnpm specs:mutate` takes about 5 minutes more and can run in the same job.
