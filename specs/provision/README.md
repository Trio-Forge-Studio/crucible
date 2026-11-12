# Provisioning model

`Provision.tla` models `crucible provision <env>` for one kind of resource in one environment: the lock file, the intent journal, adopt-before-create, a crash at any step, a create whose answer is lost after the write did or did not land, the developer editing display names between runs, and anyone renaming a resource on Roblox between runs, an unresolved create's included. It follows "Rules that keep provisioning safe" in `design/architecture.md`. `crates/iac/src/provision.rs` implements it, and `crates/iac/tests/conformance.rs` drives that code against a fake of Roblox and checks the invariants below after every step.

Run it with `cargo xtask tla`. The command needs Java 11 or later on `PATH` or in `JAVA_HOME`. It downloads TLC 1.7.4 into `target/tla` and checks the jar's SHA-256, or uses the jar `TLA2TOOLS_JAR` names.

## Names, IDs and keys

Roblox knows a resource by its ID and its display name. It never sees the key. Each remote resource in the model carries a name and a ghost owner: the key a create was sent for, or `Hand` for a resource someone made by hand. The protocol never reads the owner. The properties do, so they can say what the protocol cannot see.

- `want[k]` is the display name `crucible.toml` gives key `k` in this environment. `Rename(k, n)` changes it between runs. `Distinct` is the config rule that two keys of one kind never share a display name in one environment, and `crucible plan` enforces it.
- An intent records the name its create sent and, in `before`, the IDs its run knew before the create: the run's list, `known`, and the creates it had answers for. The provisioning journal's `before` field holds them.
- A key's adopt check takes `Candidates(k)`: the resources with its declared name and, while it has an intent, every resource whose ID is not in the intent's `before`, whatever its name. A create renamed on Roblox after its answer was lost is still found. A resource the lock binds to a key is never a candidate.
- `List` drops an inherited intent whose adopt check finds nothing: under A1 its create never landed. Provision writes that journal before anything else, and a crash before the write leaves the state `List` started from.
- No intent is written and no create sent while another key's intent is unresolved, that is, while the journal holds an intent for a key the lock has no ID for. So every resource new to an intent is the intent's own create, or one made by hand.
- One candidate is adopted. Two candidates for a key, or one candidate of two keys, stop the run (`Ambiguous`) and ask for `crucible adopt`.
- A run ends when every key is settled or has had its create, or once a create of the run lost its answer: provision stops then.
- `Refused` is the plan's refusal of a declared name that another key's unresolved intent sent. That key's create may have landed under the name, so the run stops before it lists anything.
- `Update(k)` writes the declared name to the resource the lock names.
- `RenameOnRoblox(i, n)` is anyone with access renaming a resource on Roblox between runs, within A3. It shares the rename budget with `Rename`.
- `crucible adopt` reads the resource before it writes the ID. It must be one of the key's candidates, and no other key without a lock entry may have it as a candidate.

The lock records the universe each environment's entries belong to, and `crucible plan` refuses a lock or a journal for another universe than `crucible.toml` names, two environments with one universe, and a lock table or a journal for an environment `crucible.toml` no longer declares. So the model's one `remote` is the universe both files describe, and no other environment provisions it.

## What TLC checks

| Property | In plain English | TLA+ |
| --- | --- | --- |
| `AtMostOne` | Provision makes at most one remote resource per key in the environment. | `\A k \in Keys : Cardinality(Owned(k)) <= 1` |
| `LockNamesExisting` | The lock names a resource that exists and that no other key holds: one provision made for that key, or one made by hand when provision made none. | `lock[k] # 0 => lock[k] \in Ids /\ remote[lock[k]].owner \in {k, Hand} /\ (remote[lock[k]].owner = Hand => Owned(k) = {}) /\ \A j \in Keys \ {k} : lock[j] # lock[k]` |
| `NoBlindCreate` | No create follows an unknown outcome without a list, and none runs while a resource the key would adopt exists, so the list's result is what decides. | `[][\A k \in Keys : Create(k) => k \notin unknown /\ Candidates(k) = {}]_vars` |
| `JournalCoversUnknown` | A create whose outcome the provisioner may not know always has its intent in the journal, so the next run and `crucible plan` can report it. | `\A k \in unknown : lock[k] = 0 => journal[k] # None` |
| `Converges` | Once faults and renames stop, reruns reach the declared state: every key holds its resource's ID under its declared name and no intent is left. Or the runs keep stopping on a state only the developer can resolve: two candidates for a key, one candidate of two keys, or a refused name. | `<>[](\A k \in Keys : Settled(k)) \/ <>[]Stuck` |
| `FaultFreeBinds` | With no fault, no rename and no two resources under one declared name, the runs bind every key to a resource under its declared name. | `(\A k \in Keys : Cardinality({i \in Ids : remote[i].name = want[k]}) <= 1 /\ [](faults = 0 /\ renames = 0)) => <>(\A k \in Keys : lock[k] \in Ids /\ remote[lock[k]].name = want[k])` |

`Converges` accepts runs that keep stopping, and its `Stuck` reuses the protocol's `Refused`, so a protocol that refuses every run passes it. `FaultFreeBinds` reads only the variables, so it fails such a protocol. `unknown` is a ghost variable. A create adds its key, and a list, a lock write or `crucible adopt` removes it. `Candidates(k)` reads Roblox, not the run's list result `seen`, so `NoBlindCreate` does not restate the guard `seen[k] = {}`. Under A1 and A2 the two agree only when the run uses what it listed.

`Provision.cfg` checks two keys, three names, one resource made by hand, two faults and two renames. A fault is a crash, a lost answer or a dropped lock entry. A rename is an edit of a display name in `crucible.toml` or on Roblox. Every protocol step is weakly fair. Faults, renames and `crucible adopt` are not. TLC 1.7.4 explores 490,656 states, 175,434 of them distinct, to depth 20, in under a minute on two workers. Two resources made by hand, three faults and three renames give 3,419,832 distinct states, to depth 27, in about 16 minutes with the same result.

## Assumptions

- **A1.** A create lands before its answer reaches the client, or never lands, and the list endpoint shows every landed create. Roblox documents neither. The mutant `list-misses-recent-creates` drops the second half, and TLC then finds a duplicate in nine states, whatever the journal holds.
- **A2.** One provisioner runs per environment, as rule 6 states. Nothing checks it.
- **A3.** A lost lock entry names a resource that still carries its key's declared name, and the key keeps that name until provision adopts the resource again. A lost entry is found only by its name, so nothing can find a resource whose entry and name are both gone. `Rename`, `RenameOnRoblox` and `DropLockEntry` encode it.
- **A4.** When a run stops on two candidates, the developer passes `crucible adopt` the one provision created for the key, if it created one. `AdoptById` encodes it.

An earlier model needed a fifth: nobody renames a resource on Roblox while an intent is unresolved, because the adopt check after a lost create looked only for the name the create sent and the declared one. The intent's `before` replaced that assumption. The three rules it took each have a mutant below that TLC catches. Seeking the sent name as well as the new IDs is no longer needed: TLC finds no error with it at either configuration, and the adopt check leaves it out to keep one rule. It still seeks the sent name for an intent written by hand without `before`, because such an intent has no IDs to compare.

The model leaves out changes on Roblox while a run is in progress, resources made by hand after the first run, updates of fields other than the name, retirement and key renames with `moved_from`, which are idempotent writes to a resource the lock already names, definite rejections, which create nothing, and the deletion of a resource, which Open Cloud cannot do for these kinds. Provision stops when the lock names an ID Roblox no longer has, and never creates the resource again on its own. Badge cost is out too: `expectedCost: 0` makes a paid badge create a definite rejection. Kinds are independent: each has its own list, and a name is unique only within a kind.

Badges take a stricter path than the model. Roblox lists badges through a public endpoint that may lag behind a create, which is A1 failing, so an unresolved badge intent always stops the run and asks the developer to adopt the badge or remove the intent. That only removes steps from the model's runs, and the conformance test's fake has a badge list that lags behind a lost create to check it.

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
| `adopt-check-ignores-new-ids` | The adopt check after a lost create looks only for the declared name, and the create was renamed. | `LockNamesExisting`, 8 states |
| `intent-while-another-is-unresolved` | An intent is written while another key's create may have landed, so that resource is new to it. | `NoBlindCreate`, 7 states |
| `adopt-a-shared-candidate` | A run adopts a resource another key's adopt check finds too. | `LockNamesExisting`, 10 states |
| `adopt-by-id-takes-a-shared-candidate` | `crucible adopt` accepts a resource another key without a lock entry would adopt. | `LockNamesExisting`, 8 states |

Dropping an inherited intent that the list shows never landed is load-bearing too: without it, two keys whose creates were lost before they landed each wait for the other's intent, and TLC reports a deadlock in nine states. Deadlock is not a property `mutants.toml` can name, so no mutant records it.

## What the journal buys

The intent's `before` is what finds a lost create after a rename on Roblox, and without the journal no run knows which create may have landed. With `create-before-intent` applied, TLC breaks `JournalCoversUnknown` in four states. If A1 fails, the adopt check finds nothing and a create runs again whatever the journal holds, which is why badges, whose list may lag, stop instead.

`Refused`, the plan's refusal of a name another key's unresolved intent sent, is now redundant: a resource under that name would be a candidate of both keys, and the run stops on it anyway. TLC finds no counterexample without it at either configuration. `crucible plan` keeps it, because it runs without a list and cannot see that both keys would find the resource.
