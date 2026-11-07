---------------------------- MODULE Provisioning ----------------------------
(***************************************************************************)
(* `crucible provision <env>` for one environment.                         *)
(*                                                                         *)
(* The config (`crucible.toml`), each provisioner's checkout (the lock and *)
(* the intent journal), Roblox's remote state, and the provisioner's       *)
(* steps. Every provisioner action is one operation of the Rust code: one  *)
(* Open Cloud call, one atomic file write, or one decision on the listing. *)
(* README.md maps them to the code and states the invariants in English.   *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS
    Provs,      \* provisioners, each with its own checkout of the repository
    Keys,       \* resource keys the config can declare
    Kind,       \* [Keys -> {"product", "pass", "badge"}]
    Base,       \* [Keys -> display name stem]; two keys of one kind never share one
    MovedFrom,  \* [Targets -> Keys]: the target's `moved_from`
    Vers,       \* config versions; a new version changes the name and every field
    MaxId,      \* how many resources Roblox will hold in the model
    FreeQuota,  \* free badge creates per day
    BadgeCost,  \* the `expectedCost` a badge create sends
    MaxFaults,  \* crashes, lost responses and rejections; -1 means unbounded
    MaxEdits,   \* config edits; -1 means unbounded
    ListLag,    \* TRUE when a listing can miss a resource that was just created
    LoseLocal,  \* TRUE when a checkout can discard its uncommitted lock and journal
    Handmade    \* keys whose resource may already exist, made by hand in Creations

Targets == DOMAIN MovedFrom
Lineage(k) == IF k \in Targets THEN MovedFrom[k] ELSE k
Lineages == {Lineage(k) : k \in Keys}
Ids == 1..MaxId
Statuses == {"absent", "managed", "retired", "untrack"}
Name(k, v) == <<Base[k], v>>
NoName == <<"", 0>>
NoEntry == [id |-> 0, fp |-> 0, st |-> "none"]
NoRec == [kind |-> "", name |-> NoName, active |-> FALSE, claim |-> "", vis |-> FALSE]
Fresh == [s |-> "todo", id |-> 0]
IdleRun == [pc |-> "idle", obs |-> {}, ks |-> [k \in Keys |-> Fresh]]
Clean == [st |-> "none", name |-> NoName]

VARIABLES
    cfg,        \* the config every checkout reads: [Keys -> [st, ver]]
    lock,       \* crucible.lock on each checkout's disk: [Provs -> [Keys -> entry]]
    journal,    \* .crucible/journal/<env>.json: [Provs -> [Keys -> intent name]]
    committed,  \* the lock as last committed, for LoseLocal
    remote,     \* Roblox: [Ids -> record]; resources are never deleted. The
                \* ghost field `claim` names the key that created or adopted it.
    nextId,     \* the next ID Roblox assigns
    freeBadges, \* free badge creates left today
    robux,      \* Robux spent on badges
    run,        \* each provisioner's memory, lost on a crash
    uncertain,  \* ghost: a create whose outcome no durable record settles
    blind,      \* ghost: a create ran while such an outcome was unsettled
    shadowed,   \* ghost: a create ran beside an exact match in its own listing
    faults,     \* fault budget spent
    edits       \* edit budget spent

world == <<remote, nextId, freeBadges, robux>>
vars == <<cfg, lock, journal, committed, remote, nextId, freeBadges, robux,
          run, uncertain, blind, shadowed, faults, edits>>

-----------------------------------------------------------------------------
(* Reading the config, the lock and the listing *)

DesiredName(k) == Name(k, cfg[k].ver)
DesiredSt(k) == IF cfg[k].st = "retired" THEN "retired" ELSE "managed"
MovedAway(a) == \E t \in Targets : MovedFrom[t] = a /\ cfg[t].st # "absent"
Locked(p, k) == lock[p][k].id # 0
Exists(i) == remote[i].kind # ""

\* A key the config no longer declares, whose lock entry or intent says
\* Roblox may hold a live resource for it.
Orphan(p, k) ==
    /\ cfg[k].st = "absent"
    /\ ~MovedAway(k)
    /\ lock[p][k].st = "managed" \/ journal[p][k] # NoName

Valid(p) ==
    /\ \A k \in Keys : ~Orphan(p, k)
    /\ \A t \in Targets : cfg[t].st # "absent" =>
           /\ cfg[MovedFrom[t]].st = "absent"
           /\ Locked(p, t) => ~Locked(p, MovedFrom[t]) /\ journal[p][MovedFrom[t]] = NoName

LockedIds(p) == {lock[p][k].id : k \in Keys} \ {0}
Matches(p, kind, name) ==
    {o \in run[p].obs : o.kind = kind /\ o.name = name /\ o.id \notin LockedIds(p)}

Ks(p, k) == run[p].ks[k]
Working(p) == run[p].pc = "working"
StaleIntent(p, k) == journal[p][k] # NoName /\ Ks(p, k).s = "todo"
NoStaleIntents(p) == \A k \in Keys : ~StaleIntent(p, k)

NeedsMove(p, k) ==
    /\ k \in Targets
    /\ cfg[k].st # "absent"
    /\ ~Locked(p, k)
    /\ Locked(p, MovedFrom[k])
    /\ journal[p][MovedFrom[k]] = NoName
NeedsUntrack(p, k) == cfg[k].st = "untrack" /\ Locked(p, k)
NeedsUpdate(p, k) ==
    /\ cfg[k].st \in {"managed", "retired"}
    /\ Locked(p, k)
    /\ lock[p][k].fp # cfg[k].ver \/ lock[p][k].st # DesiredSt(k)
NeedsCreate(p, k) ==
    /\ cfg[k].st = "managed"
    /\ ~Locked(p, k)
    /\ ~(k \in Targets /\ Locked(p, MovedFrom[k]))

Settled(p, k) ==
    \/ Ks(p, k).s = "unknown"
    \/ Ks(p, k).s \in {"rejected", "deferred", "failed"} /\ journal[p][k] = NoName
    \/ /\ Ks(p, k).s = "todo"
       /\ journal[p][k] = NoName
       /\ ~NeedsMove(p, k) /\ ~NeedsUntrack(p, k)
       /\ ~NeedsUpdate(p, k) /\ ~NeedsCreate(p, k)

-----------------------------------------------------------------------------
(* Helpers for the steps *)

Budget(max, n) == max < 0 \/ n < max
Spend(max, n) == IF max < 0 THEN n ELSE n + 1
Fault == Budget(MaxFaults, faults) /\ faults' = Spend(MaxFaults, faults)
SetKs(p, k, v) == run' = [run EXCEPT ![p].ks[k] = v]
Stop(p) == run' = [run EXCEPT ![p] = IdleRun]
Claim(i, k) == [remote EXCEPT ![i].claim = IF @ = "" THEN k ELSE @]
Adopted(i) == [id |-> i, fp |-> 0, st |-> "managed"]

\* An adopt check settles an unsettled create only when it searches a listing
\* taken after that create, for the name the create sent.
Checked(p, l, name) ==
    uncertain' = [uncertain EXCEPT ![p][l] =
                      IF @.st = "listed" /\ @.name = name THEN Clean ELSE @]

-----------------------------------------------------------------------------
(* The provisioner *)

\* Read the config, the lock and the journal. An invalid config stops here:
\* the command prints the error and writes nothing, so no state changes.
Start(p) ==
    /\ run[p].pc = "idle"
    /\ Valid(p)
    /\ run' = [run EXCEPT ![p].pc = "started"]
    /\ UNCHANGED <<cfg, lock, journal, committed, world, uncertain, blind, shadowed, faults, edits>>

\* The product, pass and public badge lists.
List(p) ==
    /\ run[p].pc = "started"
    /\ run' = [run EXCEPT ![p].pc = "working",
                          ![p].obs = {[id |-> i, kind |-> remote[i].kind, name |-> remote[i].name] :
                                          i \in {j \in Ids : Exists(j) /\ remote[j].vis}}]
    /\ uncertain' = [uncertain EXCEPT ![p] =
                         [l \in Lineages |->
                             IF uncertain[p][l].st = "open"
                             THEN [uncertain[p][l] EXCEPT !.st = "listed"]
                             ELSE uncertain[p][l]]]
    /\ UNCHANGED <<cfg, lock, journal, committed, world, blind, shadowed, faults, edits>>

\* An intent left by an earlier run: search the listing for the name it sent.
ResolveIntent(p, k) ==
    /\ Working(p)
    /\ StaleIntent(p, k)
    /\ ~Locked(p, k)
    /\ LET n == journal[p][k]
           M == Matches(p, Kind[k], n)
       IN /\ Checked(p, Lineage(k), n)
          /\ \/ /\ Cardinality(M) = 1
                /\ LET i == (CHOOSE o \in M : TRUE).id
                   IN /\ lock' = [lock EXCEPT ![p][k] = Adopted(i)]
                      /\ remote' = Claim(i, k)
                /\ SetKs(p, k, [s |-> "adopted", id |-> 0])
                /\ UNCHANGED journal
             \/ /\ M = {}
                /\ journal' = [journal EXCEPT ![p][k] = NoName]
                /\ UNCHANGED <<lock, remote, run>>
             \/ /\ Cardinality(M) > 1
                /\ Stop(p)
                /\ UNCHANGED <<lock, journal, remote>>
    /\ UNCHANGED <<cfg, committed, nextId, freeBadges, robux, blind, shadowed, faults, edits>>

ClearIntent(p, k) ==
    /\ Working(p)
    /\ journal[p][k] # NoName
    /\ \/ Ks(p, k).s \in {"adopted", "locked", "rejected", "deferred"}
       \/ Ks(p, k).s = "todo" /\ Locked(p, k)
    /\ journal' = [journal EXCEPT ![p][k] = NoName]
    /\ SetKs(p, k, IF Ks(p, k).s \in {"rejected", "deferred"} THEN Ks(p, k) ELSE Fresh)
    /\ UNCHANGED <<cfg, lock, committed, world, uncertain, blind, shadowed, faults, edits>>

\* `moved_from`: one lock write hands the old key's entry to the new key.
Move(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsMove(p, k)
    /\ lock' = [lock EXCEPT ![p][k] = lock[p][MovedFrom[k]],
                            ![p][MovedFrom[k]] = NoEntry]
    /\ remote' = [remote EXCEPT ![lock[p][MovedFrom[k]].id].claim =
                      IF @ = MovedFrom[k] THEN k ELSE @]
    /\ UNCHANGED <<cfg, journal, committed, nextId, freeBadges, robux, run,
                   uncertain, blind, shadowed, faults, edits>>

\* `untrack = true`: forget the ID and leave the resource as it is on Roblox.
Untrack(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsUntrack(p, k)
    /\ lock' = [lock EXCEPT ![p][k] = NoEntry]
    /\ remote' = [remote EXCEPT ![lock[p][k].id].claim = ""]
    /\ UNCHANGED <<cfg, journal, committed, nextId, freeBadges, robux, run,
                   uncertain, blind, shadowed, faults, edits>>

\* The idempotent update. Retiring is an update that takes a product or pass
\* off sale or disables a badge; restoring is the reverse.
Update(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsUpdate(p, k)
    /\ LET i == lock[p][k].id
           applied == [remote EXCEPT ![i].name = DesiredName(k),
                                     ![i].active = (DesiredSt(k) = "managed")]
       IN \/ /\ remote' = applied
             /\ SetKs(p, k, [s |-> "updated", id |-> i])
             /\ UNCHANGED faults
          \/ /\ Fault
             /\ remote' \in {remote, applied}
             /\ SetKs(p, k, [s |-> "failed", id |-> 0])
    /\ UNCHANGED <<cfg, lock, journal, committed, nextId, freeBadges, robux,
                   uncertain, blind, shadowed, edits>>

WriteLockUpdated(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "updated"
    /\ lock' = [lock EXCEPT ![p][k] = [id |-> @.id, fp |-> cfg[k].ver, st |-> DesiredSt(k)]]
    /\ SetKs(p, k, Fresh)
    /\ UNCHANGED <<cfg, journal, committed, world, uncertain, blind, shadowed, faults, edits>>

\* Before a create: adopt one exact name match, or decide to create.
AdoptCheck(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsCreate(p, k)
    /\ LET M == Matches(p, Kind[k], DesiredName(k))
       IN /\ Checked(p, Lineage(k), DesiredName(k))
          /\ \/ /\ Cardinality(M) = 1
                /\ LET i == (CHOOSE o \in M : TRUE).id
                   IN /\ lock' = [lock EXCEPT ![p][k] = Adopted(i)]
                      /\ remote' = Claim(i, k)
                /\ UNCHANGED run
             \/ /\ M = {}
                /\ SetKs(p, k, [s |-> "clear", id |-> 0])
                /\ UNCHANGED <<lock, remote>>
             \/ /\ Cardinality(M) > 1
                /\ Stop(p)
                /\ UNCHANGED <<lock, remote>>
    /\ UNCHANGED <<cfg, journal, committed, nextId, freeBadges, robux, blind, shadowed, faults, edits>>

WriteIntent(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "clear"
    /\ journal' = [journal EXCEPT ![p][k] = DesiredName(k)]
    /\ SetKs(p, k, [s |-> "intent", id |-> 0])
    /\ UNCHANGED <<cfg, lock, committed, world, uncertain, blind, shadowed, faults, edits>>

\* The create call and its outcome. Roblox commits or not; the response is
\* accepted, lost (unknown), rejected, or a refused paid badge (deferred).
Create(p, k) ==
    LET l == Lineage(k)
        n == DesiredName(k)
        open == [st |-> "open", name |-> n]
        room == nextId <= MaxId
        paid == Kind[k] = "badge" /\ freeBadges = 0
        commits == room /\ (~paid \/ BadgeCost >= 100)
    IN
    /\ Working(p)
    /\ Ks(p, k).s = "intent"
    /\ blind' = (blind \/ uncertain[p][l].st # "none")
    /\ shadowed' = (shadowed \/ Matches(p, Kind[k], n) # {})
    /\ \/ /\ commits
          /\ remote' = [remote EXCEPT ![nextId] =
                            [kind |-> Kind[k], name |-> n, active |-> TRUE,
                             claim |-> k, vis |-> ~ListLag]]
          /\ nextId' = nextId + 1
          /\ freeBadges' = IF Kind[k] = "badge" /\ ~paid THEN freeBadges - 1 ELSE freeBadges
          /\ robux' = IF paid THEN robux + BadgeCost ELSE robux
          /\ uncertain' = [uncertain EXCEPT ![p][l] = open]
          /\ \/ SetKs(p, k, [s |-> "created", id |-> nextId]) /\ UNCHANGED faults
             \/ Fault /\ SetKs(p, k, [s |-> "unknown", id |-> 0])
       \/ /\ Fault
          /\ \/ SetKs(p, k, [s |-> "unknown", id |-> 0]) /\ uncertain' = [uncertain EXCEPT ![p][l] = open]
             \/ SetKs(p, k, [s |-> "rejected", id |-> 0]) /\ UNCHANGED uncertain
          /\ UNCHANGED world
       \/ /\ ~commits
          /\ SetKs(p, k, [s |-> IF room THEN "deferred" ELSE "rejected", id |-> 0])
          /\ UNCHANGED <<world, uncertain, faults>>
    /\ UNCHANGED <<cfg, lock, journal, committed, edits>>

WriteLockCreated(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "created"
    /\ lock' = [lock EXCEPT ![p][k] = [id |-> Ks(p, k).id, fp |-> cfg[k].ver, st |-> "managed"]]
    /\ uncertain' = [uncertain EXCEPT ![p][Lineage(k)] = Clean]
    /\ SetKs(p, k, [s |-> "locked", id |-> 0])
    /\ UNCHANGED <<cfg, journal, committed, world, blind, shadowed, faults, edits>>

Finish(p) ==
    /\ Working(p)
    /\ \A k \in Keys : Settled(p, k)
    /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, world, uncertain, blind, shadowed, faults, edits>>

\* Memory is lost; the lock, the journal and Roblox keep every finished write.
Crash(p) ==
    /\ run[p].pc # "idle"
    /\ Fault
    /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, world, uncertain, blind, shadowed, edits>>

KeyStep(p, k) ==
    \/ ResolveIntent(p, k) \/ ClearIntent(p, k) \/ Move(p, k) \/ Untrack(p, k)
    \/ Update(p, k) \/ WriteLockUpdated(p, k) \/ AdoptCheck(p, k)
    \/ WriteIntent(p, k) \/ Create(p, k) \/ WriteLockCreated(p, k)

Provision(p) == Start(p) \/ List(p) \/ Finish(p) \/ \E k \in Keys : KeyStep(p, k)

-----------------------------------------------------------------------------
(* The developer and Roblox *)

Idle == \A p \in Provs : run[p].pc = "idle"

\* Any edit to one key between runs. Absent and untracked keys carry no fields.
Edit(k, st, v) ==
    /\ Idle
    /\ Budget(MaxEdits, edits)
    /\ st \in {"absent", "untrack"} => v = 1
    /\ [st |-> st, ver |-> v] # cfg[k]
    /\ cfg' = [cfg EXCEPT ![k] = [st |-> st, ver |-> v]]
    /\ edits' = Spend(MaxEdits, edits)
    /\ UNCHANGED <<lock, journal, committed, world, run, uncertain, blind, shadowed, faults>>

CommitLock(p) ==
    /\ LoseLocal
    /\ run[p].pc = "idle"
    /\ committed[p] # lock[p]
    /\ committed' = [committed EXCEPT ![p] = lock[p]]
    /\ UNCHANGED <<cfg, lock, journal, world, run, uncertain, blind, shadowed, faults, edits>>

\* `git checkout -- crucible.lock`, a fresh clone, or a deleted `.crucible`.
Discard(p) ==
    /\ LoseLocal
    /\ run[p].pc = "idle"
    /\ Fault
    /\ lock' = [lock EXCEPT ![p] = committed[p]]
    /\ journal' = [journal EXCEPT ![p] = [k \in Keys |-> NoName]]
    /\ UNCHANGED <<cfg, committed, world, run, uncertain, blind, shadowed, edits>>

NewDay ==
    /\ freeBadges < FreeQuota
    /\ freeBadges' = FreeQuota
    /\ UNCHANGED <<cfg, lock, journal, committed, remote, nextId, robux, run,
                   uncertain, blind, shadowed, faults, edits>>

Propagate(i) ==
    /\ ListLag
    /\ Exists(i)
    /\ ~remote[i].vis
    /\ remote' = [remote EXCEPT ![i].vis = TRUE]
    /\ UNCHANGED <<cfg, lock, journal, committed, nextId, freeBadges, robux, run,
                   uncertain, blind, shadowed, faults, edits>>

-----------------------------------------------------------------------------

Init ==
    /\ cfg = [k \in Keys |-> [st |-> IF k \in Targets THEN "absent" ELSE "managed", ver |-> 1]]
    /\ lock = [p \in Provs |-> [k \in Keys |-> NoEntry]]
    /\ journal = [p \in Provs |-> [k \in Keys |-> NoName]]
    /\ committed = lock
    /\ \E h \in Handmade \cup {"none"} :
           /\ remote = [i \in Ids |->
                           IF h # "none" /\ i = 1
                           THEN [kind |-> Kind[h], name |-> Name(h, 1),
                                 active |-> TRUE, claim |-> "", vis |-> TRUE]
                           ELSE NoRec]
           /\ nextId = IF h # "none" THEN 2 ELSE 1
    /\ freeBadges = FreeQuota
    /\ robux = 0
    /\ run = [p \in Provs |-> IdleRun]
    /\ uncertain = [p \in Provs |-> [l \in Lineages |-> Clean]]
    /\ blind = FALSE
    /\ shadowed = FALSE
    /\ faults = 0
    /\ edits = 0

Next ==
    \/ \E p \in Provs : Provision(p) \/ Crash(p) \/ CommitLock(p) \/ Discard(p)
    \/ \E k \in Keys, st \in Statuses, v \in Vers : Edit(k, st, v)
    \/ NewDay
    \/ \E i \in Ids : Propagate(i)

Spec == Init /\ [][Next]_vars

\* The developer reruns provision until it succeeds.
LiveSpec == Spec /\ \A p \in Provs : WF_vars(Provision(p))

-----------------------------------------------------------------------------
(* Invariants. README.md states each one in English. *)

TypeOK ==
    /\ cfg \in [Keys -> [st : Statuses, ver : Vers]]
    /\ nextId \in 1..(MaxId + 1)
    /\ freeBadges \in 0..FreeQuota
    /\ blind \in BOOLEAN
    /\ \A p \in Provs : run[p].pc \in {"idle", "started", "working"}

AtMostOneRemotePerKey ==
    \A k \in Keys : Cardinality({i \in Ids : Exists(i) /\ remote[i].claim = k}) <= 1

LockNamesExisting ==
    \A p \in Provs, k \in Keys : Locked(p, k) => remote[lock[p][k].id].kind = Kind[k]

LockInjective ==
    \A p \in Provs, a, b \in Keys :
        a # b /\ Locked(p, a) /\ Locked(p, b) => lock[p][a].id # lock[p][b].id

LockOwnsItsResource ==
    \A p \in Provs, k \in Keys : Locked(p, k) => remote[lock[p][k].id].claim = k

NoBlindCreate == ~blind

NoCreateBesideMatch == ~shadowed

NoPaidBadge == robux = 0

RemovedIsError == \A p \in Provs : run[p].pc # "idle" => Valid(p)

\* A started run always has a next step: the Rust loop never spins or hangs.
RunAdvances == \A p \in Provs : run[p].pc # "idle" => ENABLED Provision(p)

(* Action properties *)

\* A lock entry leaves only by `untrack` or by `moved_from`.
NoSilentDelete ==
    [][\A p \in Provs, k \in Keys :
          (Locked(p, k) /\ lock'[p][k] = NoEntry) =>
              \/ cfg[k].st = "untrack"
              \/ MovedAway(k)]_vars

\* Roblox takes a resource off sale only for a key the config retires.
NoSilentRetire ==
    [][\A i \in Ids :
          (Exists(i) /\ remote[i].active /\ ~remote'[i].active) =>
              \E p \in Provs, k \in Keys : lock[p][k].id = i /\ cfg[k].st = "retired"]_vars

\* `moved_from` hands the old key's ID to the new key.
MovedKeepsId ==
    [][\A p \in Provs, t \in Targets :
          (Locked(p, MovedFrom[t]) /\ lock'[p][MovedFrom[t]].id = 0 /\ cfg[t].st # "absent") =>
              lock'[p][t].id = lock[p][MovedFrom[t]].id]_vars

(* Liveness *)

\* Once the config stops changing and is valid, every managed key gets a lock
\* entry or a reported deferral.
Progress ==
    \A p \in Provs, k \in Keys :
        cfg[k].st = "managed" ~>
            \/ Locked(p, k)
            \/ Ks(p, k).s = "deferred"
            \/ cfg[k].st # "managed"
            \/ ~Valid(p)
=============================================================================
