# Provisioning model

`Provision.tla` models `crucible provision <env>` for one kind of resource in one environment: the lock file, the intent journal, adopt-before-create by display name, a crash at any step, a create whose answer is lost after the write did or did not land, and the developer editing display names between runs. It follows "Rules that keep provisioning safe" in `design/architecture.md`. No code implements it yet. PR 8 writes `provision`, and its conformance test checks each step against this model.

Run it with `cargo xtask tla`. The command needs Java 11 or later on `PATH` or in `JAVA_HOME`. It downloads TLC 1.7.4 into `target/tla` and checks the jar's SHA-256, or uses the jar `TLA2TOOLS_JAR` names.

## Names, not keys

Roblox knows a resource by its ID and its display name. It never sees the key, so the adopt check can only match names. Each remote resource in the model carries a name and a ghost owner: the key a create was sent for, or `Hand` for a resource someone made by hand. The protocol never reads the owner. The properties do, so they can say what the protocol cannot see.

- `want[k]` is the display name `crucible.toml` gives key `k` in this environment. `Rename(k, n)` changes it between runs. `Distinct` is the config rule that two keys of one kind never share a display name in one environment, and `crucible plan` enforces it.
- The journal records the name each intent sent. After an unknown outcome, the adopt check looks for that name as well as the declared one, so a rename after a lost create still finds the resource.
- A resource the lock binds to a key is never another key's candidate.
- `Refused` is the plan's refusal of a declared name that another key's unresolved intent sent. That key's create may have landed under the name, so the run stops before it lists anything.
- `Update(k)` writes the declared name to the resource the lock names.
- `crucible adopt` reads the resource before it writes the ID. It must carry a name the key seeks, no other key may hold it, and no other key's intent may have sent its name.

The lock records the universe each environment's entries belong to, and `crucible plan` refuses a lock or a journal for another universe than `crucible.toml` names. So the model's one `remote` is the universe both files describe.

## What TLC checks

| Property | In plain English | TLA+ |
| --- | --- | --- |
| `AtMostOne` | Provision makes at most one remote resource per key in the environment. | `\A k \in Keys : Cardinality(Owned(k)) <= 1` |
| `LockNamesExisting` | The lock names a resource that exists and that no other key holds: one provision made for that key, or one made by hand when provision made none. | `lock[k] # 0 => lock[k] \in Ids /\ remote[lock[k]].owner \in {k, Hand} /\ (remote[lock[k]].owner = Hand => Owned(k) = {}) /\ \A j \in Keys \ {k} : lock[j] # lock[k]` |
| `NoBlindCreate` | No create follows an unknown outcome without a list, and none runs while a resource the key would adopt exists, so the list's result is what decides. | `[][\A k \in Keys : Create(k) => k \notin unknown /\ Candidates(k) = {}]_vars` |
| `JournalCoversUnknown` | A create whose outcome the provisioner may not know always has its intent in the journal, so the next run and `crucible plan` can report it. | `\A k \in unknown : lock[k] = 0 => journal[k] # None` |
| `Converges` | Once faults and renames stop, reruns reach the declared state: every key holds its resource's ID under its declared name and no intent is left. Or the runs keep stopping on a state only the developer can resolve: two resources match a key, or the plan refuses a name. | `<>[](\A k \in Keys : Settled(k)) \/ <>[]Stuck` |

`unknown` is a ghost variable. A create adds its key, and a list, a lock write or `crucible adopt` removes it. `Candidates(k)` reads Roblox, not the run's list result `seen`, so `NoBlindCreate` does not restate the guard `seen[k] = {}`. Under A1 and A2 the two agree only when the run uses what it listed.

`Provision.cfg` checks two keys, three names, one resource made by hand, two faults and two renames. A fault is a crash, a lost answer or a dropped lock entry. Every protocol step is weakly fair. Faults, renames and `crucible adopt` are not. TLC 1.7.4 explores 198,588 states, 93,432 of them distinct, to depth 25, in about 11 seconds on two workers. Two resources made by hand, three faults and three renames give 887,040 distinct states, to depth 29, in about 1 minute 45 seconds with the same result.

## Assumptions

- **A1.** A create lands before its answer reaches the client, or never lands, and the list endpoint shows every landed create. Roblox documents neither. The mutant `list-misses-recent-creates` drops the second half, and TLC then finds a duplicate in nine states, whatever the journal holds.
- **A2.** One provisioner runs per environment, as rule 6 states. Nothing checks it.
- **A3.** A lost lock entry names a resource that still carries its key's declared name, and the key keeps that name until provision adopts the resource again. A lost entry is found only by its name, so nothing can find a resource whose entry and name are both gone. `Rename` and `DropLockEntry` encode it.
- **A4.** When a run stops on two matches, the developer passes `crucible adopt` the one provision created for the key, if it created one. `AdoptById` encodes it.

The model leaves out updates of fields other than the name, retirement and key renames with `moved_from`, which are idempotent writes to a resource the lock already names, and definite rejections, which create nothing. Badge cost is out too: `expectedCost: 0` makes a paid badge create a definite rejection. Kinds are independent: each has its own list, and a name is unique only within a kind.

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

`two-keys-one-name` and `adopt-check-ignores-the-sent-name` are the two counterexamples that a model which used the key as the name could not show: one key adopting another key's resource, and a second resource after a rename that follows a lost create. Two matches stop the run in the checked model too: an invariant that `Ambiguous` is never enabled fails in nine states, through a resource made by hand that a rename gives a key's name.

## What the journal buys

Once names can change, the journal is load-bearing under A1. With `create-before-intent` applied, TLC breaks `AtMostOne` in nine states: a create's answer is lost, its key is renamed, and the next run lists the new name, finds nothing and creates a second resource. It breaks `LockNamesExisting` in eight: another key takes the old name and adopts the first key's resource. The intent's sent name is what lets the adopt check find a resource after a rename, and what lets the plan refuse a name another key's create may hold. If A1 fails, the adopt check finds nothing and the design's rule creates again whatever the journal holds. A run that stops on an inherited intent, leaving a human to resolve it, is the safer choice for badges, whose list is a public endpoint that may be cached. PR 8 decides that.
