------------------------------ MODULE Provision ------------------------------
(***************************************************************************)
(* What `crucible provision <env>` does to one kind of resource in one     *)
(* environment: the lock file, the intent                                  *)
(* journal, adopt-before-create, crashes at any step, create responses     *)
(* that can be lost after the write either did or did not happen, creates  *)
(* Roblox refuses, resources made by hand under any name, and renames on   *)
(* Roblox between runs. design/architecture.md, "Rules that                *)
(* keep provisioning safe", is the source of each step, and                *)
(* crates/iac/src/provision.rs implements it.                              *)
(*                                                                         *)
(* Roblox knows a resource by its ID and its display name, never by its    *)
(* key. The adopt check takes the declared name, and after an unknown      *)
(* outcome stops on any ID the intent's run did not know that carries      *)
(* another name, because only a person can tell whose it is. A key's name  *)
(* changes when                                                            *)
(* the developer edits crucible.toml, and a ghost owner records which key  *)
(* a create was for, so the properties can say what the protocol cannot   *)
(* see.                                                                    *)
(*                                                                         *)
(* Four assumptions bound what TLC proves. specs/provision/README.md       *)
(* states them and what breaks without them.                              *)
(*   A1. A create either lands before its outcome reaches the client, or   *)
(*       never lands, and the list endpoint shows every landed create.     *)
(*   A2. One provisioner runs per environment (rule 6).                    *)
(*   A3. A lost lock entry names a resource that carries its key's name,   *)
(*       and that key keeps its name until provision adopts it again.      *)
(*   A4. After a run stops on two matches or on a new ID, the developer    *)
(*       adopts the one provision created for the key, if it created one,  *)
(*       and clears the intent with --none only if it created none.        *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets, Sequences

CONSTANTS
    Keys,       \* the resource keys of one kind the config declares here
    Names,      \* the display names a key may have
    MaxHand,    \* resources someone makes by hand, before the first run or between runs
    MaxFaults,  \* crashes, lost responses and dropped lock edits, in total
    MaxRenames  \* display name edits in crucible.toml or on Roblox, in total

VARIABLES
    remote,     \* Roblox: remote[i] is the resource with ID i, its name and ghost owner
    want,       \* crucible.toml: want[k] is k's display name in this environment
    lock,       \* crucible.lock: lock[k] is k's ID, or 0 when the lock has no entry
    journal,    \* .crucible/journal/<env>.json: journal[k] is the name k's intent sent, or None
    before,     \* the journal too: before[k] holds the IDs k's intent's run knew before its create
    pc,         \* the provisioner's step
    seen,       \* this run's list result: seen[k] holds the IDs k would adopt
    blocked,    \* keys this run has sent a create for (rule 1)
    reply,      \* the key and ID of a create the run has an answer for
    known,      \* the IDs this run listed or got from a create's answer
    faults,     \* faults used so far
    renames,    \* renames used so far, in crucible.toml or on Roblox
    unknown,    \* ghost: keys created with no ID in the lock and no list since
    dropped,    \* ghost: keys whose lock entry was lost and not yet restored (A3)
    rejected,   \* ghost: keys whose last create Roblox refused, and whose intent the run cleared
    how         \* ghost: how[k] says how lock[k] was bound: "create", "name", "other" or "person"

vars == <<remote, want, lock, journal, before, pc, seen, blocked, reply, known, faults, renames, unknown, dropped,
          rejected, how>>

None == "none"
Hand == "hand"
NoReply == [key |-> CHOOSE k \in Keys : TRUE, id |-> 0]

Ids == DOMAIN remote
Match(n) == {i \in Ids : remote[i].name = n}
Owned(k) == {i \in Ids : remote[i].owner = k}
Locked == {lock[k] : k \in Keys} \ {0}

\* crucible.toml rejects two keys of one kind with one display name here.
Distinct(w) == \A k, j \in Keys : k # j => w[k] # w[j]

\* An adopt check looks for the declared name and, while the key has an
\* intent, for any ID the intent's run did not know, whatever its name: the
\* create may have landed and been renamed since, or someone made it by hand.
\* A run adopts only a resource with the declared name, and stops on a new ID
\* with another name. A resource the lock binds to a key is never sought.
New(k) == IF journal[k] = None THEN {} ELSE Ids \ before[k]
Sought(k) == (Match(want[k]) \cup New(k)) \ Locked
Candidates(k) == Match(want[k]) \ Locked
Strangers(k) == Sought(k) \ Candidates(k)

\* An intent whose create may have landed: the lock holds no ID for its key.
Pending(k) == lock[k] = 0 /\ journal[k] # None
OtherPending(k) == \E j \in Keys \ {k} : Pending(j)

\* A resource two keys would adopt. Neither may take it.
Shared(k) == \E j \in Keys \ {k} : lock[j] = 0 /\ seen[k] \cap seen[j] # {}

\* `crucible plan` refuses a declared name that another key's unresolved intent
\* sent, because that key's create may have landed under it.
Refused == \E k, j \in Keys : k # j /\ lock[j] = 0 /\ journal[j] = want[k]

\* The developer edits crucible.toml between runs, keeping names distinct. A3
\* keeps a key's name while its lock entry is lost.
Rename(k, n) ==
    /\ pc \in {"down", "done"}
    /\ renames < MaxRenames
    /\ k \notin dropped
    /\ want[k] # n
    /\ want' = [want EXCEPT ![k] = n]
    /\ Distinct(want')
    /\ renames' = renames + 1
    /\ UNCHANGED <<remote, lock, journal, before, pc, seen, blocked, reply, known, faults, unknown, dropped, rejected, how>>

\* Anyone with access renames a resource on Roblox between runs, an
\* unresolved create's included. A3: not a resource whose lock entry is
\* lost, which carries its key's declared name.
RenameOnRoblox(i, n) ==
    /\ pc \in {"down", "done"}
    /\ renames < MaxRenames
    /\ \A k \in dropped : remote[i].name # want[k]
    /\ remote[i].name # n
    /\ remote' = [remote EXCEPT ![i].name = n]
    /\ renames' = renames + 1
    /\ UNCHANGED <<want, lock, journal, before, pc, seen, blocked, reply, known, faults, unknown, dropped, rejected, how>>

\* Resources someone made by hand before the first run. Two may share a name.
HandMade == UNION {[1..n -> [name : Names, owner : {Hand}]] : n \in 0..MaxHand}

\* Someone makes a resource by hand between runs, under any name, a declared
\* one or not, while a key's intent is unresolved or not.
HandMadeLater(n) ==
    /\ pc \in {"down", "done"}
    /\ Cardinality(Owned(Hand)) < MaxHand
    /\ remote' = Append(remote, [name |-> n, owner |-> Hand])
    /\ UNCHANGED <<want, lock, journal, before, pc, seen, blocked, reply, known, faults, renames, unknown, dropped,
                   rejected, how>>

Init ==
    /\ remote \in HandMade
    /\ want \in [Keys -> Names]
    /\ Distinct(want)
    /\ lock = [k \in Keys |-> 0]
    /\ journal = [k \in Keys |-> None]
    /\ before = [k \in Keys |-> {}]
    /\ pc = "down"
    /\ seen = [k \in Keys |-> {}]
    /\ blocked = {}
    /\ reply = NoReply
    /\ known = {}
    /\ faults = 0
    /\ renames = 0
    /\ unknown = {}
    /\ dropped = {}
    /\ rejected = {}
    /\ how = [k \in Keys |-> "none"]

\* A run plans first, reading the config, the lock and the journal from disk.
\* Nothing else survives a run.
Begin ==
    /\ pc \in {"down", "done"}
    /\ ~Refused
    /\ pc' = "fresh"
    /\ seen' = [k \in Keys |-> {}]
    /\ blocked' = {}
    /\ reply' = NoReply
    /\ known' = {}
    /\ UNCHANGED <<remote, want, lock, journal, before, faults, renames, unknown, dropped, rejected, how>>

Refuse ==
    /\ pc \in {"down", "done"}
    /\ Refused
    /\ pc' = "done"
    /\ UNCHANGED <<remote, want, lock, journal, before, seen, blocked, reply, known, faults, renames, unknown, dropped, rejected, how>>

\* Rule 2: before any create, list the universe's resources of this kind.
\* An inherited intent whose adopt check finds nothing goes: under A1 its
\* create never landed. Provision writes that journal before anything else,
\* and a crash before the write leaves the state this step started from.
List ==
    /\ pc = "fresh"
    /\ \A k \in Keys : lock[k] = 0 => Strangers(k) = {}
    /\ seen' = [k \in Keys |-> IF lock[k] = 0 THEN Candidates(k) ELSE {}]
    /\ journal' = [k \in Keys |-> IF Pending(k) /\ Sought(k) = {} THEN None ELSE journal[k]]
    /\ known' = Ids
    /\ unknown' = {}
    /\ pc' = "ready"
    /\ UNCHANGED <<remote, want, lock, before, blocked, reply, faults, renames, dropped, rejected, how>>

\* A resource new since a key's intent that carries another name than the
\* declared one may be the create, renamed, or one someone made by hand. The
\* plan stops on it and asks for `crucible adopt`, with its ID or --none.
Suspect ==
    /\ pc = "fresh"
    /\ \E k \in Keys : lock[k] = 0 /\ Strangers(k) # {}
    /\ pc' = "done"
    /\ UNCHANGED <<remote, want, lock, journal, before, seen, blocked, reply, known, faults, renames, unknown, dropped,
                   rejected, how>>

\* The one resource the adopt check found is adopted instead of created.
Adopt(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ Cardinality(seen[k]) = 1
    /\ ~Shared(k)
    /\ LET i == CHOOSE i \in seen[k] : TRUE
       IN /\ lock' = [lock EXCEPT ![k] = i]
          /\ how' = [how EXCEPT ![k] = IF remote[i].name = want[k] THEN "name" ELSE "other"]
    /\ journal' = [journal EXCEPT ![k] = None]
    /\ dropped' = dropped \ {k}
    /\ UNCHANGED <<remote, want, before, pc, seen, blocked, reply, known, faults, renames, unknown, rejected>>

\* Two candidates for a key, or one candidate of two keys, stop the run and
\* ask for `crucible adopt`.
Ambiguous(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ Cardinality(seen[k]) > 1 \/ Shared(k)
    /\ pc' = "done"
    /\ UNCHANGED <<remote, want, lock, journal, before, seen, blocked, reply, known, faults, renames, unknown, dropped, rejected, how>>

\* Rule 3: the intent, with the name the create sends and the IDs the run
\* knows, reaches the journal before the create is sent, and not while
\* another key's create may have landed: that resource would be new to it.
Intend(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ seen[k] = {}
    /\ journal[k] = None
    /\ ~OtherPending(k)
    /\ journal' = [journal EXCEPT ![k] = want[k]]
    /\ before' = [before EXCEPT ![k] = known]
    /\ rejected' = rejected \ {k}
    /\ UNCHANGED <<remote, want, lock, pc, seen, blocked, reply, known, faults, renames, unknown, dropped, how>>

\* Rule 1: one create per key per run. The answer arrives, or is lost after
\* the write landed, or is lost before it landed, or Roblox refuses it, which
\* makes nothing.
Create(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ k \notin blocked
    /\ seen[k] = {}
    /\ journal[k] = want[k]
    /\ blocked' = blocked \cup {k}
    /\ unknown' = unknown \cup {k}
    /\ \/ /\ remote' = Append(remote, [name |-> want[k], owner |-> k])
          /\ reply' = [key |-> k, id |-> Len(remote) + 1]
          /\ pc' = "answered"
          /\ UNCHANGED faults
       \/ /\ faults < MaxFaults
          /\ remote' = Append(remote, [name |-> want[k], owner |-> k])
          /\ faults' = faults + 1
          /\ UNCHANGED <<reply, pc>>
       \/ /\ faults < MaxFaults
          /\ faults' = faults + 1
          /\ UNCHANGED <<remote, reply, pc>>
       \/ /\ faults < MaxFaults
          /\ faults' = faults + 1
          /\ reply' = [key |-> k, id |-> 0]
          /\ pc' = "rejected"
          /\ UNCHANGED remote
    /\ UNCHANGED <<want, lock, journal, before, seen, known, renames, dropped, rejected, how>>

\* A refused create made nothing, so its intent is cleared before the run
\* stops. A crash before that write leaves the intent of a create that never
\* landed, which the next run treats as one whose answer was lost.
Abandon ==
    /\ pc = "rejected"
    /\ journal' = [journal EXCEPT ![reply.key] = None]
    /\ rejected' = rejected \cup {reply.key}
    /\ unknown' = unknown \ {reply.key}
    /\ reply' = NoReply
    /\ pc' = "done"
    /\ UNCHANGED <<remote, want, lock, before, seen, blocked, known, faults, renames, dropped, how>>

\* Rule 3: the ID reaches the lock, then the intent is cleared.
Record ==
    /\ pc = "answered"
    /\ lock' = [lock EXCEPT ![reply.key] = reply.id]
    /\ known' = known \cup {reply.id}
    /\ unknown' = unknown \ {reply.key}
    /\ dropped' = dropped \ {reply.key}
    /\ how' = [how EXCEPT ![reply.key] = "create"]
    /\ pc' = "recorded"
    /\ UNCHANGED <<remote, want, journal, before, seen, blocked, reply, faults, renames, rejected>>

Clear ==
    /\ pc = "recorded"
    /\ journal' = [journal EXCEPT ![reply.key] = None]
    /\ reply' = NoReply
    /\ pc' = "ready"
    /\ UNCHANGED <<remote, want, lock, before, seen, blocked, known, faults, renames, unknown, dropped, rejected, how>>

\* An update writes the declared name to the resource the lock names.
Update(k) ==
    /\ pc = "ready"
    /\ lock[k] # 0
    /\ remote[lock[k]].name # want[k]
    /\ remote' = [remote EXCEPT ![lock[k]].name = want[k]]
    /\ UNCHANGED <<want, lock, journal, before, pc, seen, blocked, reply, known, faults, renames, unknown, dropped, rejected, how>>

\* A crash between Record and Clear leaves an intent for a key the lock holds.
Forget(k) ==
    /\ pc = "ready"
    /\ lock[k] # 0
    /\ journal[k] # None
    /\ journal' = [journal EXCEPT ![k] = None]
    /\ UNCHANGED <<remote, want, lock, before, pc, seen, blocked, reply, known, faults, renames, unknown, dropped, rejected, how>>

Settled(k) == lock[k] # 0 /\ journal[k] = None /\ remote[lock[k]].name = want[k]

\* A run ends when each key is settled or has had its create this run, or
\* once a create of this run lost its answer: provision stops then.
Finish ==
    /\ pc = "ready"
    /\ \/ \A k \in Keys : Settled(k) \/ k \in blocked
       \/ \E k \in blocked : Pending(k)
    /\ pc' = "done"
    /\ UNCHANGED <<remote, want, lock, journal, before, seen, blocked, reply, known, faults, renames, unknown, dropped, rejected, how>>

\* `crucible adopt <env> <kind> <key> <id>` reads the resource before writing
\* its ID: the key's adopt check must seek it, and no other key's without a
\* lock entry may. A4 says which of two the developer picks.
AdoptById(k, i) ==
    /\ pc \in {"down", "done"}
    /\ lock[k] = 0
    /\ i \in Sought(k)
    /\ \A j \in Keys \ {k} : lock[j] = 0 => i \notin Sought(j)
    /\ Owned(k) # {} => i \in Owned(k)
    /\ lock' = [lock EXCEPT ![k] = i]
    /\ journal' = [journal EXCEPT ![k] = None]
    /\ unknown' = unknown \ {k}
    /\ dropped' = dropped \ {k}
    /\ how' = [how EXCEPT ![k] = "person"]
    /\ UNCHANGED <<remote, want, before, pc, seen, blocked, reply, known, faults, renames, rejected>>

\* `crucible adopt <env> <kind> <key> --none` clears an unresolved intent. It
\* refuses while a resource carries the key's declared name, which the next
\* run would adopt. A4 says the developer runs it only when provision made
\* nothing for the key.
DropIntent(k) ==
    /\ pc \in {"down", "done"}
    /\ Pending(k)
    /\ Candidates(k) = {}
    /\ Owned(k) = {}
    /\ journal' = [journal EXCEPT ![k] = None]
    /\ unknown' = unknown \ {k}
    /\ UNCHANGED <<remote, want, lock, before, pc, seen, blocked, reply, known, faults, renames, dropped, rejected,
                   how>>

\* The process dies at any step and loses everything it held in memory.
Crash ==
    /\ pc \notin {"down", "done"}
    /\ faults < MaxFaults
    /\ faults' = faults + 1
    /\ pc' = "down"
    /\ UNCHANGED <<remote, want, lock, journal, before, seen, blocked, reply, known, renames, unknown, dropped, rejected,
                   how>>

\* A lock entry is lost, as when someone discards the uncommitted lock change.
DropLockEntry(k) ==
    /\ pc \in {"down", "done"}
    /\ lock[k] # 0
    /\ remote[lock[k]].name = want[k]
    /\ faults < MaxFaults
    /\ faults' = faults + 1
    /\ lock' = [lock EXCEPT ![k] = 0]
    /\ dropped' = dropped \cup {k}
    /\ UNCHANGED <<remote, want, journal, before, pc, seen, blocked, reply, known, renames, unknown, rejected, how>>

Next ==
    \/ Begin
    \/ Refuse
    \/ List
    \/ Suspect
    \/ Abandon
    \/ Record
    \/ Clear
    \/ Finish
    \/ Crash
    \/ \E i \in Ids, n \in Names : RenameOnRoblox(i, n)
    \/ \E n \in Names : HandMadeLater(n)
    \/ \E k \in Keys :
        \/ Adopt(k) \/ Ambiguous(k) \/ Intend(k) \/ Create(k) \/ Update(k) \/ Forget(k)
        \/ DropLockEntry(k) \/ DropIntent(k)
        \/ \E n \in Names : Rename(k, n)
        \/ \E i \in 1..(Len(remote) + 1) : AdoptById(k, i)

\* Every protocol step is weakly fair. Faults, renames, resources made by
\* hand and `crucible adopt` are not, so they can stop.
Fairness ==
    /\ WF_vars(Begin)
    /\ WF_vars(List)
    /\ WF_vars(Suspect)
    /\ WF_vars(Abandon)
    /\ WF_vars(Record)
    /\ WF_vars(Clear)
    /\ WF_vars(Finish)
    /\ \A k \in Keys :
        /\ WF_vars(Adopt(k))
        /\ WF_vars(Ambiguous(k))
        /\ WF_vars(Intend(k))
        /\ WF_vars(Create(k))
        /\ WF_vars(Update(k))
        /\ WF_vars(Forget(k))

Spec == Init /\ [][Next]_vars /\ Fairness

-----------------------------------------------------------------------------

TypeOK ==
    /\ remote \in Seq([name : Names, owner : Keys \cup {Hand}])
    /\ want \in [Keys -> Names]
    /\ lock \in [Keys -> 0..Len(remote)]
    /\ journal \in [Keys -> Names \cup {None}]
    /\ before \in [Keys -> SUBSET (1..Len(remote))]
    /\ pc \in {"down", "fresh", "ready", "answered", "rejected", "recorded", "done"}
    /\ blocked \subseteq Keys
    /\ known \subseteq 1..Len(remote)
    /\ faults \in 0..MaxFaults
    /\ renames \in 0..MaxRenames
    /\ unknown \subseteq Keys
    /\ dropped \subseteq Keys
    /\ rejected \subseteq Keys
    /\ how \in [Keys -> {"none", "create", "name", "other", "person"}]

\* Provision makes at most one remote resource per key in this environment.
AtMostOne == \A k \in Keys : Cardinality(Owned(k)) <= 1

\* The lock names a resource that exists and that no other key holds: one
\* provision made for that key, or one made by hand when provision made none.
LockNamesExisting ==
    \A k \in Keys : lock[k] # 0 =>
        /\ lock[k] \in Ids
        /\ remote[lock[k]].owner \in {k, Hand}
        /\ remote[lock[k]].owner = Hand => Owned(k) = {}
        /\ \A j \in Keys \ {k} : lock[j] # lock[k]

\* No create follows an unknown outcome without a list, and none runs while a
\* resource the key would adopt exists, so the list's result is what decides.
NoBlindCreate == [][\A k \in Keys : Create(k) => k \notin unknown /\ Sought(k) = {}]_vars

\* A key is bound only to its own create, to a resource a person adopted, or
\* to one that carried the key's declared name when a run adopted it.
BoundRightly == \A k \in Keys : lock[k] # 0 => remote[lock[k]].owner = k \/ how[k] \in {"name", "person"}

\* A create whose outcome the process may not know always has its intent in the
\* journal, so the next run and `crucible plan` can say so.
JournalCoversUnknown == \A k \in unknown : lock[k] = 0 => journal[k] # None

\* A run stops and asks the developer: two resources match a key, one
\* resource matches two keys, a key's name is one another key's intent sent,
\* or a resource is new since an intent whose create may have landed. An
\* intent of a create Roblox refused is no reason to stop.
Stuck ==
    \/ Refused
    \/ \E k \in Keys : lock[k] = 0 /\ Cardinality(Candidates(k)) > 1
    \/ \E k, j \in Keys : k # j /\ lock[k] = 0 /\ lock[j] = 0 /\ Candidates(k) \cap Candidates(j) # {}
    \/ \E k \in Keys : lock[k] = 0 /\ Strangers(k) # {} /\ k \notin rejected

\* Once faults and renames stop, reruns reach the declared state: every key
\* holds the ID of its one resource under its declared name, and no intent is
\* left. Or the runs keep stopping on a state only the developer can resolve.
Converges == <>[](\A k \in Keys : Settled(k)) \/ <>[]Stuck

\* With no fault, no rename and no two resources under one declared name, the
\* runs bind every key to a resource under its declared name. It reads only
\* the variables, never the protocol's own operators, so a protocol that stops
\* or refuses when nothing is wrong fails it.
FaultFreeBinds ==
    (/\ \A k \in Keys : Cardinality({i \in DOMAIN remote : remote[i].name = want[k]}) <= 1
     /\ [](faults = 0 /\ renames = 0))
    => <>(\A k \in Keys : lock[k] \in DOMAIN remote /\ remote[lock[k]].name = want[k])

=============================================================================
