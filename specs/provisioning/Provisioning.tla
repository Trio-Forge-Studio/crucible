---------------------------- MODULE Provisioning ----------------------------
(***************************************************************************)
(* `crucible provision <env>` for one environment: the config, each        *)
(* provisioner's checkout (the lock and the intent journal), Roblox's      *)
(* resources and in-flight requests, the plan review, and the operator's   *)
(* commands. Each provisioner action is one Open Cloud call, one atomic    *)
(* file write, or one decision on data already in memory. README.md maps   *)
(* the actions to the Rust code and states every property in English.     *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS
    Provs,      \* provisioners, each with its own checkout of the repository
    Keys,       \* resource keys the config can declare
    Kind,       \* [Keys -> {"product", "badge"}]; a pass behaves as a product
    Base,       \* [Keys -> display name stem]
    MovedFrom,  \* [Targets -> Keys]: the target's `moved_from`
    Vers,       \* config versions; a new version changes the name and every field
    MaxId,      \* [Kinds -> how many resources of that kind Roblox will hold]
    PageCount,  \* pages per list endpoint
    FreeQuota,  \* free badge creates per day
    BadgeCost,  \* the `expectedCost` a badge create sends
    MaxFaults,  \* failed calls, lost responses, crashes and lost files; -1 is unbounded
    MaxEdits,   \* config edits; -1 is unbounded
    ListLag,    \* TRUE when a listing can miss a resource that was just created
    LoseLocal,  \* what a checkout can lose: its uncommitted "lock", its "journal"
    Handmade,   \* keys whose resource may already exist, made by hand in Creations
    Careful     \* TRUE when a reviewer declines a create shown beside its own orphan

Kinds == {Kind[k] : k \in Keys}
Targets == DOMAIN MovedFrom
Lineage(k) == IF k \in Targets THEN {k, MovedFrom[k]} ELSE {k}

ASSUME \A a, b \in Keys : a # b /\ Kind[a] = Kind[b] => Base[a] # Base[b]
ASSUME \A a, b \in Handmade : a # b => Kind[a] # Kind[b]
ASSUME \A t \in Targets : Kind[t] = Kind[MovedFrom[t]]

Ids == 1..(CHOOSE n \in {MaxId[x] : x \in Kinds} : \A x \in Kinds : MaxId[x] <= n)
PageSize(x) == (MaxId[x] + PageCount - 1) \div PageCount
Page(x, j) ==
    {i \in Ids : (j - 1) * PageSize(x) < i /\ (i <= j * PageSize(x) \/ j = PageCount)}
Statuses == {"absent", "managed", "retired", "untrack"}
Name(k, v) == <<Base[k], v>>
NoName == <<"", 0>>
NoEntry == [id |-> 0, fp |-> NoName, ic |-> 0, st |-> "none"]
NoRec == [ex |-> FALSE, name |-> NoName, icon |-> 0, active |-> FALSE, claim |-> "", vis |-> FALSE]
Fresh == [s |-> "todo", id |-> 0]
IdleRun == [pc |-> "idle", ok |-> {}, pages |-> [x \in Kinds |-> 0], obs |-> {},
            ks |-> [k \in Keys |-> Fresh]]

VARIABLES
    cfg,        \* the config every checkout reads: [Keys -> [st, ver]]
    lock,       \* crucible.lock on each checkout's disk: [Provs -> [Keys -> entry]]
    journal,    \* .crucible/journal/<env>.json: [Provs -> [Keys -> intent name]]
    committed,  \* the lock as last committed to git, for LoseLocal
    remote,     \* Roblox: [Kinds -> [Ids -> record]]. Resources are never deleted.
                \* The ghost field `claim` names the key that created or adopted it.
    nextId,     \* [Kinds -> the next ID Roblox assigns]
    pending,    \* creates Roblox may still commit, whose response was lost
    freeBadges, \* free badge creates left today
    robux,      \* Robux spent on badges
    run,        \* each provisioner's memory, lost on a crash
    blind,      \* ghost: a create ran while an earlier create of its key was in flight or unlocked
    shadowed,   \* ghost: a create ran beside an exact match in its own listing
    faults,     \* fault budget spent
    edits       \* edit budget spent

world == <<remote, nextId, pending, freeBadges, robux>>
ghosts == <<blind, shadowed>>
vars == <<cfg, lock, journal, committed, remote, nextId, pending,
          freeBadges, robux, run, blind, shadowed, faults, edits>>

-----------------------------------------------------------------------------
(* Reading the config, the lock and the listing *)

DesiredName(k) == Name(k, cfg[k].ver)
DesiredSt(k) == IF cfg[k].st = "retired" THEN "retired" ELSE "managed"
MovedAway(a) == \E t \in Targets : MovedFrom[t] = a /\ cfg[t].st # "absent"
Locked(p, k) == lock[p][k].id # 0
LockedIds(p, x) == {lock[p][k].id : k \in {j \in Keys : Kind[j] = x}} \ {0}

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
           /\ Locked(p, MovedFrom[t]) => journal[p][t] = NoName

Matches(p, x, name) ==
    {o \in run[p].obs : o.kind = x /\ o.name = name /\ o.id \notin LockedIds(p, x)}

Ks(p, k) == run[p].ks[k]
Working(p) == run[p].pc = "working"
ListingDone(p) == \A x \in Kinds : run[p].pages[x] = PageCount
Rank(x) == IF x = "product" THEN 1 ELSE 2
StaleIntent(p, k) == journal[p][k] # NoName /\ Ks(p, k).s = "todo"
NoStaleIntents(p) == \A k \in Keys : ~StaleIntent(p, k)

NeedsMove(p, k) ==
    /\ k \in Targets
    /\ cfg[k].st # "absent"
    /\ ~Locked(p, k)
    /\ Locked(p, MovedFrom[k])
    /\ journal[p][MovedFrom[k]] = NoName
NeedsUntrack(p, k) == cfg[k].st = "untrack" /\ Locked(p, k) /\ lock[p][k].st # "untracked"
\* A badge's icon is a separate upload, so the lock keeps its version apart.
IconOf(k) == IF Kind[k] = "badge" THEN cfg[k].ver ELSE 0
NeedsIcon(p, k) == lock[p][k].ic # IconOf(k)
NeedsUpdate(p, k) ==
    /\ cfg[k].st \in {"managed", "retired"}
    /\ Locked(p, k)
    /\ lock[p][k].fp # DesiredName(k) \/ lock[p][k].st # DesiredSt(k) \/ NeedsIcon(p, k)
Marked(p, k) == lock[p][k].fp = NoName /\ (NeedsIcon(p, k) => lock[p][k].ic = 0)
NeedsCreate(p, k) ==
    /\ cfg[k].st = "managed"
    /\ ~Locked(p, k)
    /\ k \in Targets => ~Locked(p, MovedFrom[k]) /\ journal[p][MovedFrom[k]] = NoName

\* "todo" is done once the key needs nothing. Every other settled state is a
\* result the run reports, and it makes the exit code non-zero.
Settled(p, k) ==
    \/ Ks(p, k).s \in {"unknown", "unresolved", "unapproved"}
    \/ Ks(p, k).s \in {"rejected", "deferred", "failed"} /\ journal[p][k] = NoName
    \/ /\ Ks(p, k).s = "todo"
       /\ journal[p][k] = NoName
       /\ ~NeedsMove(p, k) /\ ~NeedsUntrack(p, k)
       /\ ~NeedsUpdate(p, k) /\ ~NeedsCreate(p, k)
Unfinished(p) == {k \in Keys : Ks(p, k).s # "todo"}

-----------------------------------------------------------------------------
(* Roblox *)

Room(x) == nextId[x] <= MaxId[x]
Paid(x) == x = "badge" /\ freeBadges = 0
CanCommit(r) == Room(r.kind) /\ (~Paid(r.kind) \/ BadgeCost >= 100)
Commit(r) ==
    /\ remote' = [remote EXCEPT ![r.kind][nextId[r.kind]] =
                      [ex |-> TRUE, name |-> r.name, icon |-> r.icon, active |-> TRUE,
                       claim |-> r.claim, vis |-> ~ListLag]]
    /\ nextId' = [nextId EXCEPT ![r.kind] = @ + 1]
    /\ freeBadges' = IF r.kind = "badge" /\ ~Paid(r.kind) THEN freeBadges - 1 ELSE freeBadges
    /\ robux' = IF Paid(r.kind) THEN robux + BadgeCost ELSE robux

Claimed(k) == \E x \in Kinds, i \in Ids : remote[x][i].ex /\ remote[x][i].claim = k
InFlight(k) == \E r \in pending : r.claim = k

-----------------------------------------------------------------------------
(* Helpers for the steps *)

Budget(max, n) == max < 0 \/ n < max
Spend(max, n) == IF max < 0 THEN n ELSE n + 1
Fault == Budget(MaxFaults, faults) /\ faults' = Spend(MaxFaults, faults)
SetKs(p, k, v) == run' = [run EXCEPT ![p].ks[k] = v]
Stop(p) == run' = [run EXCEPT ![p] = IdleRun]
Adopted(i) == [id |-> i, fp |-> NoName, ic |-> 0, st |-> "managed"]
Claim(x, i, k) == [remote EXCEPT ![x][i].claim = IF @ = "" THEN k ELSE @]

\* Written apart from Matches so that a wrong Matches cannot hide its own bug.
ExactMatchShown(p, k, name) ==
    \E o \in run[p].obs :
        /\ o.kind = Kind[k] /\ o.name = name
        /\ \A j \in Keys : Kind[j] = o.kind => lock[p][j].id # o.id

-----------------------------------------------------------------------------
(* The provisioner *)

Start(p) ==
    /\ run[p].pc = "idle"
    /\ Valid(p)
    /\ run' = [run EXCEPT ![p].pc = "listing"]
    /\ UNCHANGED <<cfg, lock, journal, committed, world, ghosts, faults, edits>>

ListPage(p, x) ==
    /\ run[p].pc = "listing"
    /\ run[p].pages[x] < PageCount
    /\ \A y \in Kinds : Rank(y) < Rank(x) => run[p].pages[y] = PageCount
    /\ LET j == run[p].pages[x] + 1
           shown == {[kind |-> x, id |-> i, name |-> remote[x][i].name] :
                         i \in {i \in Page(x, j) : remote[x][i].ex /\ remote[x][i].vis}}
       IN \/ /\ run' = [run EXCEPT ![p].pages[x] = j, ![p].obs = @ \cup shown]
             /\ UNCHANGED faults
          \/ /\ Fault
             /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, world, ghosts, edits>>

\* The plan this listing gives: each create that no exact match covers,
\* shown beside every unlocked resource of its kind as a possible orphan. A
\* careful reviewer declines a create shown beside the key's own resource.
Approvable(p) ==
    {k \in Keys :
        /\ NeedsCreate(p, k)
        /\ journal[p][k] = NoName
        /\ Matches(p, Kind[k], DesiredName(k)) = {}
        /\ Careful => \A o \in run[p].obs :
                         o.kind = Kind[k] /\ o.id \notin LockedIds(p, o.kind) =>
                             remote[o.kind][o.id].claim # k}

Approve(p, all) ==
    /\ run[p].pc = "listing"
    /\ ListingDone(p)
    /\ run' = [run EXCEPT ![p].pc = "working", ![p].ok = IF all THEN Approvable(p) ELSE {}]
    /\ UNCHANGED <<cfg, lock, journal, committed, world, ghosts, faults, edits>>

\* No match settles nothing, because the create may commit later.
ResolveIntent(p, k) ==
    /\ Working(p)
    /\ StaleIntent(p, k)
    /\ ~Locked(p, k)
    /\ LET M == Matches(p, Kind[k], journal[p][k])
       IN \/ /\ Cardinality(M) = 1
             /\ LET i == (CHOOSE o \in M : TRUE).id
                IN /\ lock' = [lock EXCEPT ![p][k] = Adopted(i)]
                   /\ remote' = Claim(Kind[k], i, k)
             /\ SetKs(p, k, [s |-> "adopted", id |-> 0])
          \/ /\ M = {}
             /\ SetKs(p, k, [s |-> "unresolved", id |-> 0])
             /\ UNCHANGED <<lock, remote>>
          \/ /\ Cardinality(M) > 1
             /\ Stop(p)
             /\ UNCHANGED <<lock, remote>>
    /\ UNCHANGED <<cfg, journal, committed, nextId, pending, freeBadges, robux,
                   ghosts, faults, edits>>

ClearIntent(p, k) ==
    /\ Working(p)
    /\ journal[p][k] # NoName
    /\ \/ Ks(p, k).s \in {"adopted", "locked", "rejected", "deferred"}
       \/ Ks(p, k).s = "todo" /\ Locked(p, k)
    /\ journal' = [journal EXCEPT ![p][k] = NoName]
    /\ SetKs(p, k, IF Ks(p, k).s \in {"rejected", "deferred"} THEN Ks(p, k) ELSE Fresh)
    /\ UNCHANGED <<cfg, lock, committed, world, ghosts, faults, edits>>

Move(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsMove(p, k)
    /\ LET a == MovedFrom[k]
       IN /\ lock' = [lock EXCEPT ![p][k] = lock[p][a], ![p][a] = NoEntry]
          /\ remote' = [remote EXCEPT ![Kind[k]][lock[p][a].id].claim = IF @ = a THEN k ELSE @]
    /\ UNCHANGED <<cfg, journal, committed, nextId, pending, freeBadges, robux, run,
                   ghosts, faults, edits>>

\* The entry keeps the ID, so no adopt check can take the resource.
Untrack(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsUntrack(p, k)
    /\ lock' = [lock EXCEPT ![p][k].st = "untracked"]
    /\ UNCHANGED <<cfg, journal, committed, world, run, ghosts, faults, edits>>

\* Before an update, the lock forgets what the update will change, so a
\* crash or a failed call leaves the next run an update to do.
WriteLockStale(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsUpdate(p, k)
    /\ ~Marked(p, k)
    /\ lock' = [lock EXCEPT ![p][k].fp = NoName,
                            ![p][k].ic = IF NeedsIcon(p, k) THEN 0 ELSE @]
    /\ UNCHANGED <<cfg, journal, committed, world, run, ghosts, faults, edits>>

\* A product or pass update carries the icon. A badge's metadata update
\* cannot, so UpdateIcon follows it.
Update(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsUpdate(p, k)
    /\ Marked(p, k)
    /\ LET x == Kind[k]
           i == lock[p][k].id
           applied == [remote EXCEPT
                          ![x][i].name = DesiredName(k),
                          ![x][i].active = (DesiredSt(k) = "managed"),
                          ![x][i].icon = IF x = "badge" THEN @ ELSE cfg[k].ver]
           next == IF NeedsIcon(p, k) THEN "icon" ELSE "updated"
       IN \/ /\ remote' = applied
             /\ SetKs(p, k, [s |-> next, id |-> 0])
             /\ UNCHANGED faults
          \/ /\ Fault
             /\ remote' \in {remote, applied}
             /\ SetKs(p, k, [s |-> "failed", id |-> 0])
    /\ UNCHANGED <<cfg, lock, journal, committed, nextId, pending, freeBadges, robux,
                   ghosts, edits>>

UpdateIcon(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "icon"
    /\ LET applied == [remote EXCEPT ![Kind[k]][lock[p][k].id].icon = cfg[k].ver]
       IN \/ /\ remote' = applied
             /\ SetKs(p, k, [s |-> "updated", id |-> 0])
             /\ UNCHANGED faults
          \/ /\ Fault
             /\ remote' \in {remote, applied}
             /\ SetKs(p, k, [s |-> "failed", id |-> 0])
    /\ UNCHANGED <<cfg, lock, journal, committed, nextId, pending, freeBadges, robux,
                   ghosts, edits>>

WriteLockUpdated(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "updated"
    /\ lock' = [lock EXCEPT ![p][k] = [id |-> @.id, fp |-> DesiredName(k), ic |-> IconOf(k),
                                       st |-> DesiredSt(k)]]
    /\ SetKs(p, k, Fresh)
    /\ UNCHANGED <<cfg, journal, committed, world, ghosts, faults, edits>>

AdoptCheck(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsCreate(p, k)
    /\ LET M == Matches(p, Kind[k], DesiredName(k))
       IN \/ /\ Cardinality(M) = 1
             /\ LET i == (CHOOSE o \in M : TRUE).id
                IN /\ lock' = [lock EXCEPT ![p][k] = Adopted(i)]
                   /\ remote' = Claim(Kind[k], i, k)
             /\ UNCHANGED run
          \/ /\ M = {}
             /\ SetKs(p, k, [s |-> IF k \in run[p].ok THEN "clear" ELSE "unapproved", id |-> 0])
             /\ UNCHANGED <<lock, remote>>
          \/ /\ Cardinality(M) > 1
             /\ Stop(p)
             /\ UNCHANGED <<lock, remote>>
    /\ UNCHANGED <<cfg, journal, committed, nextId, pending, freeBadges, robux,
                   ghosts, faults, edits>>

WriteIntent(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "clear"
    /\ journal' = [journal EXCEPT ![p][k] = DesiredName(k)]
    /\ SetKs(p, k, [s |-> "intent", id |-> 0])
    /\ UNCHANGED <<cfg, lock, committed, world, ghosts, faults, edits>>

\* The create call. "rejected" is a response that proves Roblox committed
\* nothing; any other failure is "unknown", and Roblox may commit it later.
Create(p, k) ==
    LET x == Kind[k]
        req == [kind |-> x, name |-> DesiredName(k), icon |-> cfg[k].ver, claim |-> k]
    IN
    /\ Working(p)
    /\ Ks(p, k).s = "intent"
    /\ blind' = (blind \/ \E j \in Lineage(k) : InFlight(j) \/ Claimed(j))
    /\ shadowed' = (shadowed \/ ExactMatchShown(p, k, req.name))
    /\ \/ /\ CanCommit(req)
          /\ Commit(req)
          /\ SetKs(p, k, [s |-> "created", id |-> nextId[x]])
          /\ UNCHANGED <<pending, faults>>
       \/ /\ Fault
          /\ pending' = pending \cup {req}
          /\ SetKs(p, k, [s |-> "unknown", id |-> 0])
          /\ UNCHANGED <<remote, nextId, freeBadges, robux>>
       \/ /\ Fault
          /\ SetKs(p, k, [s |-> "rejected", id |-> 0])
          /\ UNCHANGED world
       \/ /\ ~CanCommit(req)
          /\ SetKs(p, k, [s |-> IF Room(x) THEN "deferred" ELSE "rejected", id |-> 0])
          /\ UNCHANGED <<world, faults>>
    /\ UNCHANGED <<cfg, lock, journal, committed, edits>>

WriteLockCreated(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "created"
    /\ lock' = [lock EXCEPT ![p][k] = [id |-> Ks(p, k).id, fp |-> DesiredName(k), ic |-> IconOf(k),
                                       st |-> "managed"]]
    /\ SetKs(p, k, [s |-> "locked", id |-> 0])
    /\ UNCHANGED <<cfg, journal, committed, world, ghosts, faults, edits>>

Finish(p) ==
    /\ Working(p)
    /\ \A k \in Keys : Settled(p, k)
    /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, world, ghosts, faults, edits>>

\* Memory is lost; the lock, the journal and Roblox keep every finished write.
Crash(p) ==
    /\ run[p].pc # "idle"
    /\ Fault
    /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, world, ghosts, edits>>

KeyStep(p, k) ==
    \/ ResolveIntent(p, k) \/ ClearIntent(p, k) \/ Move(p, k) \/ Untrack(p, k)
    \/ WriteLockStale(p, k) \/ Update(p, k) \/ UpdateIcon(p, k) \/ WriteLockUpdated(p, k)
    \/ AdoptCheck(p, k)
    \/ WriteIntent(p, k) \/ Create(p, k) \/ WriteLockCreated(p, k)

Provision(p) ==
    \/ Start(p) \/ Finish(p)
    \/ \E x \in Kinds : ListPage(p, x)
    \/ \E all \in BOOLEAN : Approve(p, all)
    \/ \E k \in Keys : KeyStep(p, k)

-----------------------------------------------------------------------------
(* The operator, the developer, git and Roblox *)

\* `crucible adopt <kind> <key> <id>`. Safe when the operator names the key's
\* own resource, or names any unlocked resource while no create of the key
\* can have committed.
OperatorAdopt(p, k, i) ==
    LET x == Kind[k]
    IN /\ run[p].pc = "idle"
       /\ cfg[k].st \in {"managed", "retired"}
       /\ ~Locked(p, k)
       /\ remote[x][i].ex
       /\ i \notin LockedIds(p, x)
       /\ \/ remote[x][i].claim = k
          \/ remote[x][i].claim = "" /\ ~InFlight(k) /\ ~Claimed(k)
       /\ lock' = [lock EXCEPT ![p][k] = Adopted(i)]
       /\ journal' = [journal EXCEPT ![p][k] = NoName]
       /\ remote' = Claim(x, i, k)
       /\ UNCHANGED <<cfg, committed, nextId, pending, freeBadges, robux, run,
                      ghosts, faults, edits>>

\* `crucible resolve <env> <key> --not-created`. Safe only when the create the
\* intent records never committed and never will.
OperatorNotCreated(p, k) ==
    /\ run[p].pc = "idle"
    /\ journal[p][k] # NoName
    /\ ~Locked(p, k)
    /\ ~InFlight(k) /\ ~Claimed(k)
    /\ journal' = [journal EXCEPT ![p][k] = NoName]
    /\ UNCHANGED <<cfg, lock, committed, world, run, ghosts, faults, edits>>

Operate(p) ==
    \/ \E k \in Keys : OperatorNotCreated(p, k)
    \/ \E k \in Keys, i \in Ids : OperatorAdopt(p, k, i)

Idle == \A p \in Provs : run[p].pc = "idle"

\* Absent and untracked keys carry no fields.
Edit(k, st, v) ==
    /\ Idle
    /\ Budget(MaxEdits, edits)
    /\ st \in {"absent", "untrack"} => v = 1
    /\ [st |-> st, ver |-> v] # cfg[k]
    /\ cfg' = [cfg EXCEPT ![k] = [st |-> st, ver |-> v]]
    /\ edits' = Spend(MaxEdits, edits)
    /\ UNCHANGED <<lock, journal, committed, world, run, ghosts, faults>>

CommitLock(p) ==
    /\ "lock" \in LoseLocal
    /\ run[p].pc = "idle"
    /\ committed[p] # lock[p]
    /\ committed' = [committed EXCEPT ![p] = lock[p]]
    /\ UNCHANGED <<cfg, lock, journal, world, run, ghosts, faults, edits>>

\* `git checkout -- crucible.lock` or a fresh clone.
DiscardLock(p) ==
    /\ "lock" \in LoseLocal
    /\ run[p].pc = "idle"
    /\ lock[p] # committed[p]
    /\ Fault
    /\ lock' = [lock EXCEPT ![p] = committed[p]]
    /\ UNCHANGED <<cfg, journal, committed, world, run, ghosts, edits>>

\* A deleted `.crucible` or a fresh clone.
DiscardJournal(p) ==
    /\ "journal" \in LoseLocal
    /\ run[p].pc = "idle"
    /\ \E k \in Keys : journal[p][k] # NoName
    /\ Fault
    /\ journal' = [journal EXCEPT ![p] = [k \in Keys |-> NoName]]
    /\ UNCHANGED <<cfg, lock, committed, world, run, ghosts, edits>>

NewDay ==
    /\ freeBadges < FreeQuota
    /\ freeBadges' = FreeQuota
    /\ UNCHANGED <<cfg, lock, journal, committed, remote, nextId, pending, robux, run,
                   ghosts, faults, edits>>

Propagate(x, i) ==
    /\ ListLag
    /\ remote[x][i].ex
    /\ ~remote[x][i].vis
    /\ remote' = [remote EXCEPT ![x][i].vis = TRUE]
    /\ UNCHANGED <<cfg, lock, journal, committed, nextId, pending, freeBadges, robux, run,
                   ghosts, faults, edits>>

\* Roblox finishes a create whose response was lost, or drops it for good.
Settle(r) ==
    /\ r \in pending
    /\ pending' = pending \ {r}
    /\ \/ CanCommit(r) /\ Commit(r)
       \/ UNCHANGED <<remote, nextId, freeBadges, robux>>
    /\ UNCHANGED <<cfg, lock, journal, committed, run, ghosts, faults, edits>>

-----------------------------------------------------------------------------

Init ==
    /\ cfg = [k \in Keys |-> [st |-> IF k \in Targets THEN "absent" ELSE "managed", ver |-> 1]]
    /\ lock = [p \in Provs |-> [k \in Keys |-> NoEntry]]
    /\ journal = [p \in Provs |-> [k \in Keys |-> NoName]]
    /\ committed = lock
    /\ \E H \in SUBSET Handmade :
           /\ remote = [x \in Kinds |-> [i \in Ids |->
                           IF i = 1 /\ \E h \in H : Kind[h] = x
                           THEN [ex |-> TRUE, name |-> Name(CHOOSE h \in H : Kind[h] = x, 1),
                                 icon |-> 1, active |-> TRUE, claim |-> "", vis |-> TRUE]
                           ELSE NoRec]]
           /\ nextId = [x \in Kinds |-> IF \E h \in H : Kind[h] = x THEN 2 ELSE 1]
    /\ pending = {}
    /\ freeBadges \in 0..FreeQuota
    /\ robux = 0
    /\ run = [p \in Provs |-> IdleRun]
    /\ blind = FALSE
    /\ shadowed = FALSE
    /\ faults = 0
    /\ edits = 0

Next ==
    \/ \E p \in Provs :
          \/ Provision(p) \/ Crash(p) \/ Operate(p)
          \/ CommitLock(p) \/ DiscardLock(p) \/ DiscardJournal(p)
    \/ \E k \in Keys, st \in Statuses, v \in Vers : Edit(k, st, v)
    \/ NewDay
    \/ \E x \in Kinds, i \in Ids : Propagate(x, i)
    \/ \E r \in pending : Settle(r)

Spec == Init /\ [][Next]_vars

\* The developer reruns provision, the reviewer approves its plan, the
\* operator resolves what a run reports, a day passes, listings catch up,
\* and Roblox finishes or drops every in-flight create.
LiveSpec ==
    /\ Spec
    /\ \A p \in Provs :
          /\ WF_vars(Provision(p))
          /\ SF_vars(Approve(p, TRUE))
          /\ SF_vars(\E k \in Keys : OperatorNotCreated(p, k))
    /\ WF_vars(NewDay)
    /\ WF_vars(\E x \in Kinds, i \in Ids : Propagate(x, i))
    /\ WF_vars(\E r \in pending : Settle(r))

-----------------------------------------------------------------------------
(* Invariants. README.md states each one in English. *)

TypeOK ==
    /\ cfg \in [Keys -> [st : Statuses, ver : Vers]]
    /\ \A x \in Kinds : nextId[x] \in 1..(MaxId[x] + 1)
    /\ freeBadges \in 0..FreeQuota
    /\ blind \in BOOLEAN
    /\ \A p \in Provs : run[p].pc \in {"idle", "listing", "working"}

AtMostOneRemotePerKey ==
    \A k \in Keys :
        Cardinality({<<x, i>> \in Kinds \X Ids : remote[x][i].ex /\ remote[x][i].claim = k}) <= 1

JournalCoversCreates ==
    \A p \in Provs, k \in Keys :
        (InFlight(k) \/ Claimed(k)) /\ ~Locked(p, k) => journal[p][k] # NoName

LockNamesExisting ==
    \A p \in Provs, k \in Keys : Locked(p, k) => remote[Kind[k]][lock[p][k].id].ex

LockInjective ==
    \A p \in Provs, a, b \in Keys :
        a # b /\ Kind[a] = Kind[b] /\ Locked(p, a) /\ Locked(p, b) => lock[p][a].id # lock[p][b].id

LockOwnsItsResource ==
    \A p \in Provs, k \in Keys : Locked(p, k) => remote[Kind[k]][lock[p][k].id].claim = k

NoBlindCreate == ~blind

NoCreateBesideMatch == ~shadowed

NoPaidBadge == robux = 0

RemovedIsError == \A p \in Provs : run[p].pc # "idle" => Valid(p)

RunAdvances == \A p \in Provs : run[p].pc # "idle" => ENABLED Provision(p)

(* Action properties *)

NoSilentDelete ==
    [][\A p \in Provs, k \in Keys :
          (Locked(p, k) /\ lock'[p][k] = NoEntry) => MovedAway(k)]_vars

NoSilentRetire ==
    [][\A x \in Kinds, i \in Ids :
          (remote[x][i].ex /\ remote[x][i].active /\ ~remote'[x][i].active) =>
              \E p \in Provs, k \in Keys :
                  Kind[k] = x /\ lock[p][k].id = i /\ cfg[k].st = "retired"]_vars

MovedKeepsId ==
    [][\A p \in Provs, t \in Targets :
          (Locked(p, MovedFrom[t]) /\ lock'[p][MovedFrom[t]].id = 0 /\ cfg[t].st # "absent") =>
              lock'[p][t].id = lock[p][MovedFrom[t]].id]_vars

(* Liveness *)

Applied(p, k) ==
    LET r == remote[Kind[k]][lock[p][k].id]
    IN /\ lock[p][k].fp = DesiredName(k)
       /\ lock[p][k].ic = IconOf(k)
       /\ lock[p][k].st = DesiredSt(k)
       /\ r.name = DesiredName(k)
       /\ r.icon = cfg[k].ver
       /\ r.active = (DesiredSt(k) = "managed")

Converged(p) ==
    \A k \in Keys :
        /\ journal[p][k] = NoName
        /\ cfg[k].st = "managed" => Locked(p, k)
        /\ cfg[k].st \in {"managed", "retired"} /\ Locked(p, k) => Applied(p, k)
        /\ cfg[k].st = "untrack" => lock[p][k].st \in {"none", "untracked"}
        /\ cfg[k].st = "absent" => lock[p][k].st # "managed"

Converges == \A p \in Provs : <>[](Converged(p) \/ ~Valid(p))
=============================================================================
