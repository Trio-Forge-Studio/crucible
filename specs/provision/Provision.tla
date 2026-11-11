------------------------------ MODULE Provision ------------------------------
(***************************************************************************)
(* What `crucible provision <env>` does to one environment, as PR 8 will   *)
(* implement it: the lock file, the intent journal, adopt-before-create,   *)
(* crashes at any step, and create responses that can be lost after the    *)
(* write either did or did not happen. design/architecture.md, "Rules that *)
(* keep provisioning safe", is the source of each step.                    *)
(*                                                                         *)
(* Two assumptions bound what TLC proves. specs/provision/README.md states *)
(* them and what breaks without them.                                      *)
(*   A1. A create either lands before its outcome reaches the client, or   *)
(*       never lands, and the list endpoint shows every landed create.     *)
(*   A2. One provisioner runs per environment (rule 6).                    *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets, Sequences

CONSTANTS
    Keys,       \* the resource keys the config declares in this environment
    MaxFaults   \* crashes, lost responses and dropped lock edits, in total

VARIABLES
    remote,     \* Roblox: the resource with ID i carries the name of key remote[i]
    lock,       \* crucible.lock: lock[k] is k's ID, or 0 when the lock has no entry
    journal,    \* .crucible/journal/<env>.json: the keys with a create intent
    pc,         \* the provisioner's step
    seen,       \* this run's list result: seen[k] holds the IDs named k
    blocked,    \* keys this run has sent a create for (rule 1)
    reply,      \* the key and ID of a create the run has an answer for
    faults,     \* faults used so far
    unknown     \* ghost: keys created with no ID in the lock and no list since

vars == <<remote, lock, journal, pc, seen, blocked, reply, faults, unknown>>

NoReply == [key |-> CHOOSE k \in Keys : TRUE, id |-> 0]

Ids == DOMAIN remote
Named(k) == {i \in Ids : remote[i] = k}

\* Resources someone made by hand before the first run, at most one per key.
Preexisting ==
    {s \in UNION {[1..n -> Keys] : n \in 0..Cardinality(Keys)} :
        \A i, j \in DOMAIN s : i # j => s[i] # s[j]}

Init ==
    /\ remote \in Preexisting
    /\ lock = [k \in Keys |-> 0]
    /\ journal = {}
    /\ pc = "down"
    /\ seen = [k \in Keys |-> {}]
    /\ blocked = {}
    /\ reply = NoReply
    /\ faults = 0
    /\ unknown = {}

\* A run reads the lock and the journal from disk. Nothing else survives a run.
Begin ==
    /\ pc \in {"down", "done"}
    /\ pc' = "fresh"
    /\ seen' = [k \in Keys |-> {}]
    /\ blocked' = {}
    /\ reply' = NoReply
    /\ UNCHANGED <<remote, lock, journal, faults, unknown>>

\* Rule 2: before any create, list the universe's resources.
List ==
    /\ pc = "fresh"
    /\ seen' = [k \in Keys |-> Named(k)]
    /\ unknown' = {}
    /\ pc' = "ready"
    /\ UNCHANGED <<remote, lock, journal, blocked, reply, faults>>

\* An exact name match is adopted instead of created.
Adopt(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ Cardinality(seen[k]) = 1
    /\ lock' = [lock EXCEPT ![k] = CHOOSE i \in seen[k] : TRUE]
    /\ journal' = journal \ {k}
    /\ UNCHANGED <<remote, pc, seen, blocked, reply, faults, unknown>>

\* Two matches stop the run and ask for `crucible adopt`.
Ambiguous(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ Cardinality(seen[k]) > 1
    /\ pc' = "done"
    /\ UNCHANGED <<remote, lock, journal, seen, blocked, reply, faults, unknown>>

\* Rule 3: the intent reaches the journal before the create is sent.
Intend(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ seen[k] = {}
    /\ k \notin journal
    /\ journal' = journal \cup {k}
    /\ UNCHANGED <<remote, lock, pc, seen, blocked, reply, faults, unknown>>

\* Rule 1: one create per key per run. The answer arrives, or is lost after
\* the write landed, or is lost before it landed.
Create(k) ==
    /\ pc = "ready"
    /\ lock[k] = 0
    /\ k \notin blocked
    /\ seen[k] = {}
    /\ k \in journal
    /\ blocked' = blocked \cup {k}
    /\ unknown' = unknown \cup {k}
    /\ \/ /\ remote' = Append(remote, k)
          /\ reply' = [key |-> k, id |-> Len(remote) + 1]
          /\ pc' = "answered"
          /\ UNCHANGED faults
       \/ /\ faults < MaxFaults
          /\ remote' = Append(remote, k)
          /\ faults' = faults + 1
          /\ UNCHANGED <<reply, pc>>
       \/ /\ faults < MaxFaults
          /\ faults' = faults + 1
          /\ UNCHANGED <<remote, reply, pc>>
    /\ UNCHANGED <<lock, journal, seen>>

\* Rule 3: the ID reaches the lock, then the intent is cleared.
Record ==
    /\ pc = "answered"
    /\ lock' = [lock EXCEPT ![reply.key] = reply.id]
    /\ unknown' = unknown \ {reply.key}
    /\ pc' = "recorded"
    /\ UNCHANGED <<remote, journal, seen, blocked, reply, faults>>

Clear ==
    /\ pc = "recorded"
    /\ journal' = journal \ {reply.key}
    /\ reply' = NoReply
    /\ pc' = "ready"
    /\ UNCHANGED <<remote, lock, seen, blocked, faults, unknown>>

\* A crash between Record and Clear leaves an intent for a key the lock holds.
Forget(k) ==
    /\ pc = "ready"
    /\ lock[k] # 0
    /\ k \in journal
    /\ journal' = journal \ {k}
    /\ UNCHANGED <<remote, lock, pc, seen, blocked, reply, faults, unknown>>

Finish ==
    /\ pc = "ready"
    /\ \A k \in Keys : (lock[k] # 0 /\ k \notin journal) \/ k \in blocked
    /\ pc' = "done"
    /\ UNCHANGED <<remote, lock, journal, seen, blocked, reply, faults, unknown>>

\* `crucible adopt <kind> <key> <id>` reads the ID before writing it.
AdoptById(k, i) ==
    /\ pc \in {"down", "done"}
    /\ lock[k] = 0
    /\ i \in Named(k)
    /\ lock' = [lock EXCEPT ![k] = i]
    /\ journal' = journal \ {k}
    /\ unknown' = unknown \ {k}
    /\ UNCHANGED <<remote, pc, seen, blocked, reply, faults>>

\* The process dies at any step and loses everything it held in memory.
Crash ==
    /\ pc \notin {"down", "done"}
    /\ faults < MaxFaults
    /\ faults' = faults + 1
    /\ pc' = "down"
    /\ UNCHANGED <<remote, lock, journal, seen, blocked, reply, unknown>>

\* A lock entry is lost, as when someone discards the uncommitted lock change.
DropLockEntry(k) ==
    /\ pc \in {"down", "done"}
    /\ lock[k] # 0
    /\ faults < MaxFaults
    /\ faults' = faults + 1
    /\ lock' = [lock EXCEPT ![k] = 0]
    /\ UNCHANGED <<remote, journal, pc, seen, blocked, reply, unknown>>

Next ==
    \/ Begin
    \/ List
    \/ Record
    \/ Clear
    \/ Finish
    \/ Crash
    \/ \E k \in Keys :
        \/ Adopt(k) \/ Ambiguous(k) \/ Intend(k) \/ Create(k) \/ Forget(k)
        \/ DropLockEntry(k)
        \/ \E i \in 1..(Len(remote) + 1) : AdoptById(k, i)

\* Every protocol step is weakly fair. Faults are not, so they can stop.
Fairness ==
    /\ WF_vars(Begin)
    /\ WF_vars(List)
    /\ WF_vars(Record)
    /\ WF_vars(Clear)
    /\ WF_vars(Finish)
    /\ \A k \in Keys :
        /\ WF_vars(Adopt(k))
        /\ WF_vars(Intend(k))
        /\ WF_vars(Create(k))
        /\ WF_vars(Forget(k))

Spec == Init /\ [][Next]_vars /\ Fairness

-----------------------------------------------------------------------------

TypeOK ==
    /\ remote \in Seq(Keys)
    /\ lock \in [Keys -> 0..Len(remote)]
    /\ journal \subseteq Keys
    /\ pc \in {"down", "fresh", "ready", "answered", "recorded", "done"}
    /\ blocked \subseteq Keys
    /\ faults \in 0..MaxFaults
    /\ unknown \subseteq Keys

\* At most one remote resource exists per key in this environment.
AtMostOne == \A k \in Keys : Cardinality(Named(k)) <= 1

\* The lock never names a resource that does not exist, or one of another key.
LockNamesExisting == \A k \in Keys : lock[k] # 0 => lock[k] \in Ids /\ remote[lock[k]] = k

\* No create follows an unknown outcome without an adopt check: a create for a
\* key never runs while an earlier create of it is unanswered and unlisted.
NoBlindCreate == [][\A k \in Keys : Create(k) => k \notin unknown]_vars

\* A create whose outcome the process may not know always has its intent in the
\* journal, so the next run and `crucible plan` can say so.
JournalCoversUnknown == \A k \in unknown : lock[k] = 0 => k \in journal

\* Once faults stop, reruns reach the declared state: every key holds the ID of
\* its one resource, and no intent is left.
Converges == <>[](\A k \in Keys : lock[k] # 0 /\ k \notin journal)

=============================================================================
