# Provisioning model

`Provision.tla` models `crucible provision <env>` for one kind of resource in one environment: the lock file, the intent journal, adopt-before-create by display name, a crash at any step, a create whose answer is lost after the write did or did not land, the developer editing display names between runs, and anyone renaming a resource on Roblox between runs. It follows "Rules that keep provisioning safe" in `design/architecture.md`. No code implements it yet. PR 8 writes `provision`, and its conformance test checks each step against this model.

Run it with `cargo xtask tla`. The command needs Java 11 or later on `PATH` or in `JAVA_HOME`. It downloads TLC 1.7.4 into `target/tla` and checks the jar's SHA-256, or uses the jar `TLA2TOOLS_JAR` names.

## Names, not keys

Roblox knows a resource by its ID and its display name. It never sees the key, so the adopt check can only match names. Each remote resource in the model carries a name and a ghost owner: the key a create was sent for, or `Hand` for a resource someone made by hand. The protocol never reads the owner. The properties do, so they can say what the protocol cannot see.

- `want[k]` is the display name `crucible.toml` gives key `k` in this environment. `Rename(k, n)` changes it between runs. `Distinct` is the config rule that two keys of one kind never share a display name in one environment, and `crucible plan` enforces it.
- The journal records the name each intent sent. After an unknown outcome, the adopt check looks for that name as well as the declared one, so a rename after a lost create still finds the resource.
- A resource the lock binds to a key is never another key's candidate.
- `Refused` is the plan's refusal of a declared name that another key's unresolved intent sent. That key's create may have landed under the name, so the run stops before it lists anything.
- `Update(k)` writes the declared name to the resource the lock names.
- `RenameOnRoblox(i, n)` is anyone with access renaming a resource on Roblox between runs, within A3 and A5. It shares the rename budget with `Rename`.
- `crucible adopt` reads the resource before it writes the ID. It must carry a name the key seeks, no other key may hold it, no other key's intent may have sent its name, and no other key without a lock entry may declare its name. The last condition mirrors the plan's refusal, and only renames on Roblox need it: without it, a deeper run (two resources made by hand, three faults, three renames) binds a key to another key's resource in 16 states. Key `a`'s resource is renamed on Roblox to `b`'s name, `b`'s intent sends that name and its create never lands, `a` takes the name in `crucible.toml` and loses its lock entry, and `crucible adopt` binds `b` to `a`'s resource by the name `b`'s intent sent.

The lock records the universe each environment's entries belong to, and `crucible plan` refuses a lock or a journal for another universe than `crucible.toml` names, two environments with one universe, and a lock table or a journal for an environment `crucible.toml` no longer declares. So the model's one `remote` is the universe both files describe, and no other environment provisions it.

## What TLC checks

| Property | In plain English | TLA+ |
| --- | --- | --- |
| `AtMostOne` | Provision makes at most one remote resource per key in the environment. | `\A k \in Keys : Cardinality(Owned(k)) <= 1` |
| `LockNamesExisting` | The lock names a resource that exists and that no other key holds: one provision made for that key, or one made by hand when provision made none. | `lock[k] # 0 => lock[k] \in Ids /\ remote[lock[k]].owner \in {k, Hand} /\ (remote[lock[k]].owner = Hand => Owned(k) = {}) /\ \A j \in Keys \ {k} : lock[j] # lock[k]` |
| `NoBlindCreate` | No create follows an unknown outcome without a list, and none runs while a resource the key would adopt exists, so the list's result is what decides. | `[][\A k \in Keys : Create(k) => k \notin unknown /\ Candidates(k) = {}]_vars` |
| `JournalCoversUnknown` | A create whose outcome the provisioner may not know always has its intent in the journal, so the next run and `crucible plan` can report it. | `\A k \in unknown : lock[k] = 0 => journal[k] # None` |
| `Converges` | Once faults and renames stop, reruns reach the declared state: every key holds its resource's ID under its declared name and no intent is left. Or the runs keep stopping on a state only the developer can resolve: two resources match a key, or the plan refuses a name. | `<>[](\A k \in Keys : Settled(k)) \/ <>[]Stuck` |
| `FaultFreeBinds` | With no fault, no rename and no two resources under one declared name, the runs bind every key to a resource under its declared name. | `(\A k \in Keys : Cardinality({i \in Ids : remote[i].name = want[k]}) <= 1 /\ [](faults = 0 /\ renames = 0)) => <>(\A k \in Keys : lock[k] \in Ids /\ remote[lock[k]].name = want[k])` |

`Converges` accepts runs that keep stopping, and its `Stuck` reuses the protocol's `Refused`, so a protocol that refuses every run passes it. `FaultFreeBinds` reads only the variables, so it fails such a protocol. `unknown` is a ghost variable. A create adds its key, and a list, a lock write or `crucible adopt` removes it. `Candidates(k)` reads Roblox, not the run's list result `seen`, so `NoBlindCreate` does not restate the guard `seen[k] = {}`. Under A1 and A2 the two agree only when the run uses what it listed.

`Provision.cfg` checks two keys, three names, one resource made by hand, two faults and two renames. A fault is a crash, a lost answer or a dropped lock entry. A rename is an edit of a display name in `crucible.toml` or on Roblox. Every protocol step is weakly fair. Faults, renames and `crucible adopt` are not. TLC 1.7.4 explores 345,804 states, 134,352 of them distinct, to depth 21, in about 20 seconds on two workers. Two resources made by hand, three faults and three renames give 1,798,128 distinct states, to depth 27, in about 5 minutes 20 seconds with the same result.

## Assumptions

- **A1.** A create lands before its answer reaches the client, or never lands, and the list endpoint shows every landed create. Roblox documents neither. The mutant `list-misses-recent-creates` drops the second half, and TLC then finds a duplicate in nine states, whatever the journal holds.
- **A2.** One provisioner runs per environment, as rule 6 states. Nothing checks it.
- **A3.** A lost lock entry names a resource that still carries its key's declared name, and the key keeps that name until provision adopts the resource again. A lost entry is found only by its name, so nothing can find a resource whose entry and name are both gone. `Rename`, `RenameOnRoblox` and `DropLockEntry` encode it.
- **A4.** When a run stops on two matches, the developer passes `crucible adopt` the one provision created for the key, if it created one. `AdoptById` encodes it.
- **A5.** Nobody renames a resource of the kind in the universe on Roblox while an intent is unresolved, that is, while the journal holds an intent for a key the lock has no ID for. After a lost answer, the adopt check finds the resource only by the name the create sent or the name now declared, so a rename on Roblox in that window hides it. `RenameOnRoblox` encodes it. The mutant `rename-on-roblox-while-an-intent-is-unresolved` drops it, and TLC finds a duplicate in ten states: a create lands and its answer is lost, someone renames the resource on Roblox, and the next run finds nothing under the names it seeks and creates a second one. Renames on Roblox at other times are in the model: of a resource the lock names, which `Update` renames back, and of one made by hand. "For PR 8" below says how provision can drop A5.

The model leaves out changes on Roblox while a run is in progress, resources made by hand after the first run, updates of fields other than the name, retirement and key renames with `moved_from`, which are idempotent writes to a resource the lock already names, and definite rejections, which create nothing. Badge cost is out too: `expectedCost: 0` makes a paid badge create a definite rejection. Kinds are independent: each has its own list, and a name is unique only within a kind.

## Each property catches its bug

`mutants.toml` holds the mutants. Each changes one line of the model, and `cargo xtask tla` fails unless TLC reports a counterexample to the property that mutant names. The safety traces are the shortest TLC finds. A lasso's length varies with how the workers interleave.

| Mutant | Bug | Counterexample |
| --- | --- | --- |
| `create-ignores-the-list` | A create runs although the list found the key's resource. | `AtMostOne`, 9 states |
| `adopt-trusts-the-typed-id` | `crucible adopt` writes the ID it is given without reading the resource. | `LockNamesExisting`, 2 states |
| `create-again-after-unknown` | A run sends a second create after the first one's answer was lost. | `NoBlindCreate`, 6 states |
| `list-result-ignored` | A list runs, and the run creates without looking at what it found. | `NoBlindCreate`, 5 states |
| `create-before-intent` | The create is sent before its intent reaches the journal. | `JournalCoversUnknown`, 4 states |
| `blocked-for-good` | Rule 1 holds per environment instead of per run. | `Converges`, a lasso |
| `list-misses-recent-creates` | Assumption A1 fails. | `AtMostOne`, 9 states |
| `two-keys-one-name` | Two keys of one kind share a display name. | `LockNamesExisting`, 5 states |
| `adopt-check-ignores-the-sent-name` | The adopt check after a lost create looks only for the declared name, and the name changed. | `AtMostOne`, 11 states |
| `adopt-with-two-candidates` | The adopt check takes one of two matches instead of stopping the run. | `LockNamesExisting`, 10 states |
| `plan-allows-a-sent-name` | The plan lets a key take the name another key's intent sent. | `LockNamesExisting`, 11 states |
| `adopt-ignores-other-intents` | `crucible adopt` accepts a resource whose name another key's intent sent. | `LockNamesExisting`, 9 states |
| `rename-on-roblox-while-an-intent-is-unresolved` | Assumption A5 fails: the resource of a lost create is renamed on Roblox. | `AtMostOne`, 10 states |
| `refuse-every-run` | The plan refuses every run, so nothing is provisioned. `Converges` still holds, because a refused run counts as stuck. | `FaultFreeBinds`, a lasso |

`two-keys-one-name` and `adopt-check-ignores-the-sent-name` are the two counterexamples that a model which used the key as the name could not show: one key adopting another key's resource, and a second resource after a rename that follows a lost create. Two matches stop the run in the checked model too: an invariant that `Ambiguous` is never enabled fails in nine states, through a resource made by hand that a rename gives a key's name.

## What the journal buys

Once names can change, the journal is load-bearing under A1. With `create-before-intent` applied, TLC breaks `AtMostOne` in nine states: a create's answer is lost, the key or its resource is renamed, and the next run finds nothing under the declared name and creates a second resource. It breaks `LockNamesExisting` in seven states through a rename on Roblox, which A5 allows because no intent was written, and in eight through a rename in `crucible.toml`: another key's name lands on the first key's resource, and that key adopts it. The intent's sent name is what lets the adopt check find a resource after a rename, and what lets the plan refuse a name another key's create may hold. If A1 fails, the adopt check finds nothing and the design's rule creates again whatever the journal holds. A run that stops on an inherited intent, leaving a human to resolve it, is the safer choice for badges, whose list is a public endpoint that may be cached. PR 8 decides that.

## For PR 8: drop A5

A5 holds only by convention, so provision should find the resource of a lost create whatever its name. The intent can record which IDs of the kind existed before its create: the run's list and the creates it has answers for. The next run's adopt check then takes the resource under the declared name or any resource the lock does not hold whose ID is not in that set. `crucible plan` ignores intent fields it does not know, so PR 8 can add the IDs without a format bump.

A variant of this model checks the idea with renames on Roblox at any time between runs, so without A5. It holds with three rules, each of which TLC shows is needed:

- The adopt check after an unknown outcome looks for the declared name and the new IDs, not the name the intent sent. With the sent name kept, a deeper run binds a key to another key's resource in 14 states, after a rename on Roblox gives that resource the sent name.
- No intent is written and no create sent while another key's intent is unresolved. Without that, another key's create is new to every intent written before it: TLC finds a create while such a resource exists in 7 states, and binds the wrong key after that key loses its lock entry in 11.
- A resource that is a candidate of two keys stops the run, like two matches. Without that, TLC binds the wrong key in 10 states: a lost create's resource is renamed on Roblox to another key's name, and that key adopts it.

With the three, TLC finds no error at the configuration of `Provision.cfg`, 162,144 distinct states to depth 20, or at two resources made by hand, three faults and three renames, 2,731,530 distinct states to depth 27. Dropping the new IDs from the adopt check gives a duplicate in 11 states. PR 8 brings the variant into this model with the journal field.
