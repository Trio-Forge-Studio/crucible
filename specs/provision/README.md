# Provisioning model

`Provision.tla` models `crucible provision <env>` for one kind of resource in one environment: the lock file, the intent journal, adopt-before-create, a crash at any step, a create whose answer is lost after the write did or did not land, a create Roblox refuses, the developer editing display names between runs, anyone renaming a resource on Roblox between runs, an unresolved create's included, and anyone making a resource by hand under any name, before the first run or between runs. It follows "Rules that keep provisioning safe" in `design/architecture.md`. `crates/iac/src/provision.rs` implements it, and `crates/iac/tests/conformance.rs` drives that code against a fake of Roblox and checks the invariants below after each run, rename and resource made by hand, and `NoBlindCreate` at each create call.

Run it with `cargo xtask tla`. The command needs Java 11 or later on `PATH` or in `JAVA_HOME`. It downloads TLC 1.7.4 into `target/tla` and checks the jar's SHA-256, or uses the jar `TLA2TOOLS_JAR` names.

## Names, IDs and keys

Roblox knows a resource by its ID and its display name. It never sees the key. Each remote resource in the model carries a name and a ghost owner: the key a create was sent for, or `Hand` for a resource someone made by hand. The protocol never reads the owner. The properties do, so they can say what the protocol cannot see.

- `want[k]` is the display name `crucible.toml` gives key `k` in this environment. `Rename(k, n)` changes it between runs. `Distinct` is the config rule that two keys of one kind never share a display name in one environment, and `crucible plan` enforces it.
- An intent records the name its create sent and, in `before`, the IDs its run knew before the create: the run's list, `known`, and the creates it had answers for. The provisioning journal's `before` field holds them.
- A key's adopt check looks at `Sought(k)`: the resources with its declared name and, while it has an intent, every resource whose ID is not in the intent's `before`, whatever its name. A resource the lock binds to a key is never sought. A run adopts only from `Candidates(k)`, those with the declared name. The rest, `Strangers(k)`, are new since the intent and carry another name: the create renamed on Roblox after its answer was lost, or a resource someone made by hand, and nothing the protocol reads tells them apart. One stops the run before it writes anything (`Suspect`) and asks for `crucible adopt` with its ID, or with `--none`.
- `List` drops an inherited intent whose adopt check finds nothing: under A1 its create never landed. Provision writes that journal before anything else, and a crash before the write leaves the state `List` started from.
- No intent is written and no create sent while another key's intent is unresolved, that is, while the journal holds an intent for a key the lock has no ID for. So every resource new to an intent is the intent's own create, or one made by hand.
- One candidate is adopted. Two candidates for a key, or one candidate of two keys, stop the run (`Ambiguous`) and ask for `crucible adopt`.
- A create Roblox refuses made nothing. `Abandon` clears its intent and the run stops. A crash before `Abandon` leaves the intent, which the next run treats as a create whose answer was lost: that only adds stops.
- `HandMadeLater(n)` is anyone making a resource by hand between runs, under any name. `MaxHand` bounds the resources made by hand, before the first run and after.
- A run ends when every key is settled or has had its create, or once a create of the run lost its answer: provision stops then.
- `Refused` is the plan's refusal of a declared name that another key's unresolved intent sent. That key's create may have landed under the name, so the run stops before it lists anything.
- `Update(k)` writes the declared name to the resource the lock names.
- `RenameOnRoblox(i, n)` is anyone with access renaming a resource on Roblox between runs, within A3. It shares the rename budget with `Rename`.
- `crucible adopt` reads the resource before it writes the ID. The key's adopt check must seek it, and no other key's without a lock entry may. `DropIntent(k)` is `crucible adopt <env> <kind> <key> --none`: it clears an unresolved intent, and refuses while a resource carries the key's declared name, which the next run would adopt.

The lock records the universe each environment's entries belong to, and `crucible plan` refuses a lock or a journal for another universe than `crucible.toml` names, two environments with one universe, and a lock table or a journal for an environment `crucible.toml` no longer declares. So the model's one `remote` is the universe both files describe, and no other environment provisions it.

## What TLC checks

| Property | In plain English | TLA+ |
| --- | --- | --- |
| `AtMostOne` | Provision makes at most one remote resource per key in the environment. | `\A k \in Keys : Cardinality(Owned(k)) <= 1` |
| `LockNamesExisting` | The lock names a resource that exists and that no other key holds: one provision made for that key, or one made by hand when provision made none. | `lock[k] # 0 => lock[k] \in Ids /\ remote[lock[k]].owner \in {k, Hand} /\ (remote[lock[k]].owner = Hand => Owned(k) = {}) /\ \A j \in Keys \ {k} : lock[j] # lock[k]` |
| `BoundRightly` | A key is bound only to its own create, to a resource a person adopted, or to one that carried the key's declared name when a run adopted it. A resource made by hand under another name is never taken on the protocol's own judgement. | `\A k \in Keys : lock[k] # 0 => remote[lock[k]].owner = k \/ how[k] \in {"name", "person"}` |
| `NoBlindCreate` | No create follows an unknown outcome without a list, and none runs while a resource the key's adopt check would see exists, so the list's result is what decides. | `[][\A k \in Keys : Create(k) => k \notin unknown /\ Sought(k) = {}]_vars` |
| `JournalCoversUnknown` | A create whose outcome the provisioner may not know always has its intent in the journal, so the next run and `crucible plan` can report it. | `\A k \in unknown : lock[k] = 0 => journal[k] # None` |
| `Converges` | Once faults and renames stop, reruns reach the declared state: every key holds its resource's ID under its declared name and no intent is left. Or the runs keep stopping on a state only the developer can resolve: two candidates for a key, one candidate of two keys, a refused name, or a resource new since an intent whose create may have landed. A create Roblox refused is no such state. | `<>[](\A k \in Keys : Settled(k)) \/ <>[]Stuck` |
| `FaultFreeBinds` | With no fault, no rename and no two resources under one declared name, the runs bind every key to a resource under its declared name. | `(\A k \in Keys : Cardinality({i \in Ids : remote[i].name = want[k]}) <= 1 /\ [](faults = 0 /\ renames = 0)) => <>(\A k \in Keys : lock[k] \in Ids /\ remote[lock[k]].name = want[k])` |

`Converges` accepts runs that keep stopping, and its `Stuck` reuses the protocol's `Refused`, so a protocol that refuses every run passes it. `FaultFreeBinds` reads only the variables, so it fails such a protocol. `unknown`, `rejected` and `how` are ghost variables. A create adds its key to `unknown`, and a list, a lock write, an `Abandon` or `crucible adopt` removes it. `Abandon` adds a key to `rejected`, and its next intent removes it, so `Stuck` can tell a stop on a create that may have landed from one on a create Roblox refused. `how[k]` records how the lock entry of `k` was bound: by its create, by a run's adopt of a resource with the declared name or another, or by a person. `Sought(k)` reads Roblox, not the run's list result `seen`, so `NoBlindCreate` does not restate the guard `seen[k] = {}`. Under A1 and A2 the two agree only when the run uses what it listed.

`Provision.cfg` checks two keys, three names, so one name is always undeclared, one resource made by hand before the first run or between runs, two faults and one rename. A fault is a crash, a lost answer, a refused create or a dropped lock entry. A rename is an edit of a display name in `crucible.toml` or on Roblox. Every protocol step is weakly fair. Faults, renames, resources made by hand and `crucible adopt` are not. TLC 1.7.4 explores 676,920 states, 290,592 of them distinct, to depth 22, in under a minute on two workers, and `cargo xtask tla` with the mutants takes about two and a half minutes. Two renames, the bound before resources could be made by hand later, give 2,348,862 states, 757,284 of them distinct, to depth 23, in about two minutes, with the same result, and every mutant below is caught there too.

## Assumptions

- **A1.** A create lands before its answer reaches the client, or never lands, and the list endpoint shows every landed create. A create that Roblox answers with a 4xx status made nothing, which is why a refused create clears its intent. Roblox documents none of the three. The mutant `list-misses-recent-creates` drops the second half, and TLC then finds a duplicate in nine states, whatever the journal holds.
- **A2.** One provisioner runs per environment, as rule 6 states. Nothing checks it.
- **A3.** A lost lock entry names a resource that still carries its key's declared name, and the key keeps that name until provision adopts the resource again. A lost entry is found only by its name, so nothing can find a resource whose entry and name are both gone. `Rename`, `RenameOnRoblox` and `DropLockEntry` encode it.
- **A4.** When a run stops on two candidates or on a resource new since an intent, the developer passes `crucible adopt` the one provision created for the key, if it created one, and runs `crucible adopt --none` only if provision created none. `AdoptById` and `DropIntent` encode it.

An earlier model needed a fifth: nobody renames a resource on Roblox while an intent is unresolved, because the adopt check after a lost create looked only for the name the create sent and the declared one. The intent's `before` replaced that assumption. The three rules it took each have a mutant below that TLC catches. A later model adopted every new ID on its own, which bound a resource someone made by hand under another name while an intent was unresolved. The mutant `adopt-a-new-id-of-another-name` restores that, and TLC breaks `BoundRightly`. Adopting a new ID with the name the intent sent, when that is not the declared name, binds the wrong resource too: the create never lands, the developer renames the key, and someone makes a resource by hand under the old name. So a run adopts only the declared name, and the code stops on any other, the sent name included. For an intent written by hand without `before`, the code takes a resource with the sent name as new, because such an intent has no IDs to compare.

The model leaves out changes on Roblox while a run is in progress, updates of fields other than the name, retirement and key renames with `moved_from`, which are idempotent writes to a resource the lock already names, the experience settings, which are one idempotent update of the universe that creates nothing and whose lock entry holds no ID, and the deletion of a resource, which Open Cloud cannot do for these kinds. A retired key whose resource is gone sends nothing. Provision stops when the lock names an ID Roblox no longer has, and never creates the resource again on its own. Badge cost is a refused create: `expectedCost: 0` makes Roblox refuse a paid badge, which clears the intent like any refused create, so the next day's run creates it. Kinds are independent: each has its own list, and a name is unique only within a kind.

Badges take a stricter path than the model. Roblox lists badges through a public endpoint that may lag behind a create, which is A1 failing, so an unresolved badge intent always stops the run and asks the developer to adopt the badge or clear the intent with `crucible adopt --none`. That only removes steps from the model's runs, and the conformance test's fake has a badge list that lags behind a lost create to check it.

## Each property catches its bug

`mutants.toml` holds the mutants. Each changes one line of the model, and `cargo xtask tla` fails unless TLC reports a counterexample to the property that mutant names. The safety traces are the shortest TLC finds. A lasso's length varies with how the workers interleave.

| Mutant | Bug | Counterexample |
| --- | --- | --- |
| `create-ignores-the-list` | A create runs although the list found the key's resource. | `AtMostOne`, 9 states |
| `adopt-trusts-the-typed-id` | `crucible adopt` writes the ID it is given without reading the resource. | `LockNamesExisting`, 2 states |
| `create-again-after-unknown` | A run sends a second create for a key after the first one's answer was lost. | `NoBlindCreate`, 6 states |
| `list-result-ignored` | A list runs, and the run creates without looking at what it found. | `NoBlindCreate`, 5 states |
| `create-before-intent` | The create is sent before its intent reaches the journal. | `JournalCoversUnknown`, 4 states |
| `blocked-for-good` | Rule 1 holds per environment instead of per run. | `Converges`, a lasso |
| `list-misses-recent-creates` | Assumption A1 fails. | `AtMostOne`, 9 states |
| `two-keys-one-name` | Two keys of one kind share a display name, so a resource with it is a candidate of both and every run stops. | `FaultFreeBinds`, a lasso |
| `adopt-with-two-candidates` | The adopt check takes one of two candidates instead of stopping the run. | `LockNamesExisting`, 10 states |
| `refuse-every-run` | The plan refuses every run, so nothing is provisioned. `Converges` still holds, because a refused run counts as stuck. | `FaultFreeBinds`, a lasso |
| `adopt-check-ignores-new-ids` | The adopt check after a lost create looks only for the declared name, and the create was renamed, so the run creates the resource again. | `AtMostOne`, 11 states |
| `intent-while-another-is-unresolved` | An intent is written while another key's create may have landed, so that resource is new to it. | `NoBlindCreate`, 7 states |
| `adopt-by-id-takes-a-shared-candidate` | `crucible adopt` accepts a resource another key without a lock entry would adopt. | `LockNamesExisting`, 8 states |
| `a-refused-create-keeps-its-intent` | A create Roblox refused keeps its intent, so a resource someone makes by hand afterwards stops every run, though nothing was created. | `Converges`, a lasso |
| `adopt-a-new-id-of-another-name` | After an unknown outcome, a run adopts a resource new since the intent that carries another name, such as one made by hand. | `BoundRightly`, 9 states |

A run's stop on one candidate of two keys (`Shared`) is now redundant too. Two keys' declared names differ, so a resource both adopt checks see is new since one key's intent and carries that key's name or neither, and the stranger stop catches it first. TLC finds no counterexample to `LockNamesExisting` without it. The code keeps the check, because its message names both keys.

Dropping an inherited intent that the list shows never landed is load-bearing too: without it, two keys whose creates were lost before they landed each wait for the other's intent, and TLC reports a deadlock in nine states. Deadlock is not a property `mutants.toml` can name, so no mutant records it.

## What the journal buys

The intent's `before` is what keeps a run from creating again after a lost create was renamed on Roblox, and without the journal no run knows which create may have landed. With `create-before-intent` applied, TLC breaks `JournalCoversUnknown` in four states. If A1 fails, the adopt check finds nothing and a create runs again whatever the journal holds, which is why badges, whose list may lag, stop instead.

`Refused`, the plan's refusal of a name another key's unresolved intent sent, is now redundant: a resource under that name would be a candidate of both keys, and the run stops on it anyway. TLC finds no counterexample without it at either configuration. `crucible plan` keeps it, because it runs without a list and cannot see that both keys would find the resource.
