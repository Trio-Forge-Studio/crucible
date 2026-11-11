------------------------------ MODULE Provision ------------------------------
(***************************************************************************)
(* What `crucible provision <env>` does to one kind of resource in one     *)
(* environment, as PR 8 will implement it: the lock file, the intent       *)
(* journal, adopt-before-create by display name, crashes at any step, and  *)
(* create responses that can be lost after the write either did or did not *)
(* happen. design/architecture.md, "Rules that keep provisioning safe", is  *)
(* the source of each step.                                                *)
(*                                                                         *)
(* Roblox knows a resource by its ID and its display name, never by its    *)
(* key, so the adopt check can only match names. A key's name changes when *)
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
(*   A4. After a run stops on two matches, the developer adopts the one    *)
(*       provision created for the key, if it created one.                 *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets, Sequences

CONSTANTS
    Keys,       \* the resource keys of one kind the config declares here
    Names,      \* the display names a key may have
    MaxHand,    \* resources someone made by hand before the first run
    MaxFaults,  \* crashes, lost responses and dropped lock edits, in total
    MaxRenames  \* display name edits in crucible.toml, in total

VARIABLES
    remote,     \* Roblox: remote[i] is the resource with ID i, its name and ghost owner
    want,       \* crucible.toml: want[k] is k's display name in this environment
    lock,       \* crucible.lock: lock[k] is k's ID, or 0 when the lock has no entry
    journal,    \* .crucible/journal/<env>.json: journal[k] is the name k's intent sent, or None
    pc,         \* the provisioner's step
    seen,       \* this run's list result: seen[k] holds the IDs k would adopt
    blocked,    \* keys this run has sent a create for (rule 1)
    reply,      \* the key and ID of a create the run has an answer for
    faults,     \* faults used so far
    renames,    \* renames used so far
    unknown,    \* ghost: keys created with no ID in the lock and no list since
    dropped     \* ghost: keys whose lock entry was lost and not yet restored (A3)

vars == <<remote, want, lock, journal, pc, seen, blocked, reply, faults, renames, unknown, dropped>>

None == "none"
Hand == "hand"
NoReply == [key |-> CHOOSE k \in Keys : TRUE, id |-> 0]

Ids == DOMAIN remote
Match(n) == {i \in Ids : remote[i].name = n}
Owned(k) == {i \in Ids : remote[i].owner = k}
Locked == {lock[k] : k \in Keys} \ {0}

\* crucible.toml rejects two keys of one kind with one display name here.
Distinct(w) == \A k, j \in Keys : k # j => w[k] # w[j]

\* The names an adopt check looks for: the declared one, and the one an
\* inherited intent sent. A resource the lock binds to a key is no candidate.
Sought(k) == {want[k]} \cup (IF journal[k] = None THEN {} ELSE {journal[k]})
Candidates(k) == UNION {Match(n) : n \in Sought(k)} \ Locked

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
    /\ UNCHANGED <<remote, lock, journal, pc, seen, blocked, reply, faults, unknown, dropped>>

\* Resources someone made by hand before the first run. Two may share a name.
HandMade == UNION {[1..n -> [name : Names, owner : {Hand}]] : n \in 0..MaxHand}

Init ==
    /\ remote \in HandMade
    /\ want \in [Keys -> Names]
    /\ Distinct(want)
    /\ lock = [k \in Keys |-> 0]
    /\ journal = [k \in Keys |-> None]
    /\ pc = "down"
    /\ seen = [k \in Keys |-> {}]
    /\ blocked = {}
    /\ reply = NoReply
    /\ faults = 0
    /\ renames = 0
    /\ unknown = {}
    /\ dropped = {}

\* A run plans first, reading the config, the lock and the journal from disk.
\* Nothing else survives a run.
Begin ==
    /\ pc \in {"down", "done"}
    /\ ~Refused
    /\ pc' = "fresh"
    /\ seen' = [k \in Keys |-> {}]
    /\ blocked' = {}
    /\ reply' = NoReply
    /\ UNCHANGED <<remote, want, lock, journal, faults, renames, unknown, dropped>>

Refuse ==
    /\ pc \in {"down", "done"}
    /\ Refused
    /\ pc' = "done"
    /\ UNCHANGED <<remote, want, lock, journal, seen, blocked, reply, faults, renames, unknown, dropped>>

\* Rule 2: before any create, list the universe's resources of this kind.
List ==
    /\ pc = "fresh"
    /\ seen' = [k \in Keys |-> IF lock[k] = 0 THEN Candidates(k) ELSE {}]
    /\ unknown' = {}
    /\ pc' = "ready"
    /\ UNCHANGED <<remote, want, lock, journal, blocked, reply, faults, renames, dropped>>

\* An exact name match is adopted instead of created.
Adopt(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ Cardinality(seen[k]) = 1
    /\ lock' = [lock EXCEPT ![k] = CHOOSE i \in seen[k] : TRUE]
    /\ journal' = [journal EXCEPT ![k] = None]
    /\ dropped' = dropped \ {k}
    /\ UNCHANGED <<remote, want, pc, seen, blocked, reply, faults, renames, unknown>>

\* Two matches stop the run and ask for `crucible adopt`.
Ambiguous(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ Cardinality(seen[k]) > 1
    /\ pc' = "done"
    /\ UNCHANGED <<remote, want, lock, journal, seen, blocked, reply, faults, renames, unknown, dropped>>

\* Rule 3: the intent, with the name the create sends, reaches the journal
\* before the create is sent. It replaces an inherited intent whose name the
\* list did not find.
Intend(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ seen[k] = {}
    /\ journal[k] # want[k]
    /\ journal' = [journal EXCEPT ![k] = want[k]]
    /\ UNCHANGED <<remote, want, lock, pc, seen, blocked, reply, faults, renames, unknown, dropped>>

\* Rule 1: one create per key per run. The answer arrives, or is lost after
\* the write landed, or is lost before it landed.
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
    /\ UNCHANGED <<want, lock, journal, seen, renames, dropped>>

\* Rule 3: the ID reaches the lock, then the intent is cleared.
Record ==
    /\ pc = "answered"
    /\ lock' = [lock EXCEPT ![reply.key] = reply.id]
    /\ unknown' = unknown \ {reply.key}
    /\ dropped' = dropped \ {reply.key}
    /\ pc' = "recorded"
    /\ UNCHANGED <<remote, want, journal, seen, blocked, reply, faults, renames>>

Clear ==
    /\ pc = "recorded"
    /\ journal' = [journal EXCEPT ![reply.key] = None]
    /\ reply' = NoReply
    /\ pc' = "ready"
    /\ UNCHANGED <<remote, want, lock, seen, blocked, faults, renames, unknown, dropped>>

\* An update writes the declared name to the resource the lock names.
Update(k) ==
    /\ pc = "ready"
    /\ lock[k] # 0
    /\ remote[lock[k]].name # want[k]
    /\ remote' = [remote EXCEPT ![lock[k]].name = want[k]]
    /\ UNCHANGED <<want, lock, journal, pc, seen, blocked, reply, faults, renames, unknown, dropped>>

\* A crash between Record and Clear leaves an intent for a key the lock holds.
Forget(k) ==
    /\ pc = "ready"
    /\ lock[k] # 0
    /\ journal[k] # None
    /\ journal' = [journal EXCEPT ![k] = None]
    /\ UNCHANGED <<remote, want, lock, pc, seen, blocked, reply, faults, renames, unknown, dropped>>

Settled(k) == lock[k] # 0 /\ journal[k] = None /\ remote[lock[k]].name = want[k]

Finish ==
    /\ pc = "ready"
    /\ \A k \in Keys : Settled(k) \/ k \in blocked
    /\ pc' = "done"
    /\ UNCHANGED <<remote, want, lock, journal, seen, blocked, reply, faults, renames, unknown, dropped>>

\* `crucible adopt <kind> <key> <id>` reads the resource before writing its ID:
\* it must carry a name the key seeks, no other key may hold it, and no other
\* key's intent may have sent its name. A4 says which of two matches the
\* developer picks.
AdoptById(k, i) ==
    /\ pc \in {"down", "done"}
    /\ lock[k] = 0
    /\ i \in Candidates(k)
    /\ \A j \in Keys \ {k} : i \notin Match(journal[j])
    /\ Owned(k) # {} => i \in Owned(k)
    /\ lock' = [lock EXCEPT ![k] = i]
    /\ journal' = [journal EXCEPT ![k] = None]
    /\ unknown' = unknown \ {k}
    /\ dropped' = dropped \ {k}
    /\ UNCHANGED <<remote, want, pc, seen, blocked, reply, faults, renames>>

\* The process dies at any step and loses everything it held in memory.
Crash ==
    /\ pc \notin {"down", "done"}
    /\ faults < MaxFaults
    /\ faults' = faults + 1
    /\ pc' = "down"
    /\ UNCHANGED <<remote, want, lock, journal, seen, blocked, reply, renames, unknown, dropped>>

\* A lock entry is lost, as when someone discards the uncommitted lock change.
DropLockEntry(k) ==
    /\ pc \in {"down", "done"}
    /\ lock[k] # 0
    /\ remote[lock[k]].name = want[k]
    /\ faults < MaxFaults
    /\ faults' = faults + 1
    /\ lock' = [lock EXCEPT ![k] = 0]
    /\ dropped' = dropped \cup {k}
    /\ UNCHANGED <<remote, want, journal, pc, seen, blocked, reply, renames, unknown>>

Next ==
    \/ Begin
    \/ Refuse
    \/ List
    \/ Record
    \/ Clear
    \/ Finish
    \/ Crash
    \/ \E k \in Keys :
        \/ Adopt(k) \/ Ambiguous(k) \/ Intend(k) \/ Create(k) \/ Update(k) \/ Forget(k)
        \/ DropLockEntry(k)
        \/ \E n \in Names : Rename(k, n)
        \/ \E i \in 1..(Len(remote) + 1) : AdoptById(k, i)

\* Every protocol step is weakly fair. Faults, renames and `crucible adopt`
\* are not, so they can stop.
Fairness ==
    /\ WF_vars(Begin)
    /\ WF_vars(List)
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
    /\ pc \in {"down", "fresh", "ready", "answered", "recorded", "done"}
    /\ blocked \subseteq Keys
    /\ faults \in 0..MaxFaults
    /\ renames \in 0..MaxRenames
    /\ unknown \subseteq Keys
    /\ dropped \subseteq Keys

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
NoBlindCreate == [][\A k \in Keys : Create(k) => k \notin unknown /\ Candidates(k) = {}]_vars

\* A create whose outcome the process may not know always has its intent in the
\* journal, so the next run and `crucible plan` can say so.
JournalCoversUnknown == \A k \in unknown : lock[k] = 0 => journal[k] # None

\* A run stops and asks the developer: two resources match a key, or a key's
\* name is one another key's intent sent.
Stuck == Refused \/ \E k \in Keys : lock[k] = 0 /\ Cardinality(Candidates(k)) > 1

\* Once faults and renames stop, reruns reach the declared state: every key
\* holds the ID of its one resource under its declared name, and no intent is
\* left. Or the runs keep stopping on a state only the developer can resolve.
Converges == <>[](\A k \in Keys : Settled(k)) \/ <>[]Stuck

=============================================================================
