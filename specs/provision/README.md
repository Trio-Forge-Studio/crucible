# Provisioning model

`Provision.tla` models `crucible provision <env>` for one environment: the lock file, the intent journal, adopt-before-create, a crash at any step, and a create whose answer is lost after the write did or did not land. It follows "Rules that keep provisioning safe" in `design/architecture.md`. No code implements it yet. PR 8 writes `provision`, and its conformance test checks each step against this model.

Run it with `cargo xtask tla`. The command needs Java 11 or later on `PATH` or in `JAVA_HOME`. It downloads TLC 1.7.4 into `target/tla` and checks the jar's SHA-256, or uses the jar `TLA2TOOLS_JAR` names.

## What TLC checks

| Property | In plain English | TLA+ |
| --- | --- | --- |
| `AtMostOne` | At most one remote resource exists per key in the environment. | `\A k \in Keys : Cardinality(Named(k)) <= 1` |
| `LockNamesExisting` | The lock never names a resource that does not exist, or one that belongs to another key. | `\A k \in Keys : lock[k] # 0 => lock[k] \in Ids /\ remote[lock[k]] = k` |
| `NoBlindCreate` | No create follows an unknown outcome without an adopt check: a key is never created while an earlier create of it has no ID in the lock and no list has run since. | `[][\A k \in Keys : Create(k) => k \notin unknown]_vars` |
| `JournalCoversUnknown` | A create whose outcome the provisioner may not know always has its intent in the journal, so the next run and `crucible plan` can report it. | `\A k \in unknown : lock[k] = 0 => k \in journal` |
| `Converges` | Once faults stop, reruns reach the declared state: every key holds its resource's ID in the lock and no intent is left. | `<>[](\A k \in Keys : lock[k] # 0 /\ k \notin journal)` |

`unknown` is a ghost variable. A create adds its key, and a list, a lock write or `crucible adopt` removes it. The provisioner never reads it, so `NoBlindCreate` does not restate the guard that enforces it.

`Provision.cfg` checks three keys and five faults. A fault is a crash, a lost answer or a dropped lock entry. Every protocol step is weakly fair and faults are not. Any key may start with one resource made by hand. TLC 1.7.4 explores 90,428 states, 32,378 of them distinct, to depth 23, in about 7 seconds on two workers. Four keys and four faults give 314,061 distinct states in about 40 seconds with the same result.

## Assumptions

- **A1.** A create lands before its answer reaches the client, or never lands, and the list endpoint shows every landed create. Roblox documents neither. The mutant `list-misses-recent-creates` drops the second half, and TLC then finds a duplicate in five states, whatever the journal holds.
- **A2.** One provisioner runs per environment, as rule 6 states. Nothing checks it.

The model leaves out updates, retirement and renames, which are idempotent writes to a resource the lock already names, and definite rejections, which create nothing. Badge cost is out too: `expectedCost: 0` makes a paid badge create a definite rejection.

## Each property catches its bug

`mutants.toml` holds one mutant per property. Each changes one line of the model, and `cargo xtask tla` fails unless TLC reports a counterexample to the property that mutant names. The safety traces are the shortest TLC finds. A lasso's length varies with how the workers interleave.

| Mutant | Bug | Counterexample |
| --- | --- | --- |
| `create-ignores-the-list` | A create runs although the list found the key's resource. | `AtMostOne`, 9 states |
| `adopt-trusts-the-typed-id` | `crucible adopt` writes the ID it is given without reading it. | `LockNamesExisting`, 2 states |
| `create-again-after-unknown` | A run sends a second create after the first one's answer was lost. | `NoBlindCreate`, 6 states |
| `create-before-intent` | The create is sent before its intent reaches the journal. | `JournalCoversUnknown`, 4 states |
| `blocked-for-good` | Rule 1 holds per environment instead of per run. | `Converges`, a lasso |
| `list-misses-recent-creates` | Assumption A1 fails. | `AtMostOne`, 5 states |

## What the journal buys

Under A1, the adopt check before every create is what keeps the environment at one resource per key. With `create-before-intent` applied, TLC still finds no counterexample to `AtMostOne`, `LockNamesExisting`, `NoBlindCreate` or `Converges`. The journal carries `JournalCoversUnknown`, the report of an outcome nobody knows. It becomes a safety mechanism only if A1 fails and a run stops on an intent it inherited instead of creating after an empty list. PR 8 decides that.
