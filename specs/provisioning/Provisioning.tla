---------------------------- MODULE Provisioning ----------------------------
(***************************************************************************)
(* Resource identity in `crucible provision <env>` for one environment:    *)
(* the config, each provisioner's lock and intent journal, Roblox's        *)
(* resources and in-flight requests, the plan review and plan files, and   *)
(* the operator's commands. Each provisioner action is one Open Cloud      *)
(* call, one atomic file write, or one decision on data already in memory. *)
(* `specs/reconcile` models how a locked resource's fields converge.       *)
(* README.md maps the actions to the Rust code and states every property.  *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS
    Provs,
    Keys,
    Kind,
    Name0,
    Names,
    MovedFrom,
    Nonces,
    Envs,
    Env,
    MaxId,
    Handmade,
    PageCount,
    FreeQuota,   \* free badge creates Roblox allows per day
    BadgeCost,   \* the `expectedCost` a badge create sends
    MaxFaults,
    MaxEdits,
    ListLag,
    LateUpdates,
    SettleFirst,
    LoseLocal,
    PlanFiles,
    Careful

Kinds == {Kind[k] : k \in Keys}
Targets == DOMAIN MovedFrom
Lineage(k) == IF k \in Targets THEN {k, MovedFrom[k]} ELSE {k}

ASSUME \A a, b \in Keys \ Targets : a # b /\ Kind[a] = Kind[b] => Name0[a] # Name0[b]
ASSUME \A t \in Targets : Kind[t] = Kind[MovedFrom[t]]
ASSUME Env \in Envs

Ids == 1..(CHOOSE n \in {MaxId[x] : x \in Kinds} : \A x \in Kinds : MaxId[x] <= n)
PageSize(x) == (MaxId[x] + PageCount - 1) \div PageCount
Page(x, j) ==
    {i \in Ids : (j - 1) * PageSize(x) < i /\ (i <= j * PageSize(x) \/ j = PageCount)}
Statuses == {"absent", "managed", "retired", "untrack"}
NoName == ""
NoIntent == "none"
NoMark == [key |-> "", n |-> NoIntent]
Mark(k, n) == [key |-> k, n |-> n]
NoEntry == [id |-> 0, st |-> "none"]
NoRec == [ex |-> FALSE, name |-> NoName, mark |-> NoMark, on |-> FALSE, vis |-> FALSE]
NoPlan == [env |-> "none"]
Fresh == [s |-> "todo", id |-> 0]
IdleRun == [pc |-> "idle", mode |-> "none", ok |-> {}, pages |-> [x \in Kinds |-> 0],
            obs |-> {}, ks |-> [k \in Keys |-> Fresh], op |-> [k |-> NoName, i |-> 0]]

VARIABLES
    cfg,
    lock,
    journal,
    committed,
    planFile,
    remote,     \* Roblox never deletes a product, pass or badge
    nextId,
    pending,
    late,
    freeBadges,
    robux,
    run,
    owner,
    blind,
    shadowed,
    faults,
    edits

roblox == <<remote, nextId, pending, late, freeBadges, robux>>
ghosts == <<owner, blind, shadowed>>
vars == <<cfg, lock, journal, committed, planFile, remote, nextId, pending, late,
          freeBadges, robux, run, owner, blind, shadowed, faults, edits>>

-----------------------------------------------------------------------------

Declared(k) == cfg[k].st \in {"managed", "retired"}
MovedAway(a) == \E t \in Targets : MovedFrom[t] = a /\ cfg[t].st # "absent"
Locked(p, k) == lock[p][k].id # 0
LockedIds(p, x) == {lock[p][k].id : k \in {j \in Keys : Kind[j] = x}} \ {0}
Intent(p, k) == journal[p][k] # NoIntent

Orphan(p, k) ==
    /\ cfg[k].st = "absent"
    /\ ~MovedAway(k)
    /\ lock[p][k].st = "managed" \/ Intent(p, k)

Valid(p) ==
    /\ \A k \in Keys : ~Orphan(p, k)
    /\ \A t \in Targets : cfg[t].st # "absent" =>
           /\ cfg[MovedFrom[t]].st = "absent"
           /\ Locked(p, t) => ~Locked(p, MovedFrom[t]) /\ ~Intent(p, MovedFrom[t])
           /\ Locked(p, MovedFrom[t]) => ~Intent(p, t)

Unlocked(p, x) == {o \in run[p].obs : o.kind = x /\ o.id \notin LockedIds(p, x)}
Marked(p, k) == {o \in Unlocked(p, Kind[k]) : o.mark.key \in Lineage(k)}
Named(p, k) == {o \in Unlocked(p, Kind[k]) : o.mark = NoMark /\ o.name = cfg[k].name}

Ks(p, k) == run[p].ks[k]
Working(p) == run[p].pc = "working"
ListingDone(p) == \A x \in Kinds : run[p].pages[x] = PageCount
Rank(x) == IF x = "product" THEN 1 ELSE 2
StaleIntent(p, k) == Intent(p, k) /\ Ks(p, k).s = "todo"
NoStaleIntents(p) == \A k \in Keys : ~StaleIntent(p, k)

Reconciled(p, k) ==
    /\ Locked(p, k)
    /\ Declared(k) \/ (cfg[k].st = "absent" /\ lock[p][k].st = "retired")
DesiredOn(k) == cfg[k].st = "managed"
View(p, k) == {o \in run[p].obs : o.kind = Kind[k] /\ o.id = lock[p][k].id}
Unlisted(p, k) == View(p, k) = {}
Drift(p, k) ==
    \/ Unlisted(p, k)
    \/ \E o \in View(p, k) :
           \/ o.on # DesiredOn(k)
           \/ Declared(k) /\ (o.name # cfg[k].name \/ o.mark # NoMark)

NeedsMove(p, k) ==
    /\ k \in Targets
    /\ cfg[k].st # "absent"
    /\ ~Locked(p, k)
    /\ Locked(p, MovedFrom[k])
    /\ ~Intent(p, MovedFrom[k])
NeedsUntrack(p, k) == cfg[k].st = "untrack" /\ Locked(p, k) /\ lock[p][k].st # "untracked"
NeedsManagedMark(p, k) ==
    /\ Locked(p, k)
    /\ \/ Declared(k) /\ lock[p][k].st = "untracked"
       \/ cfg[k].st = "managed" /\ lock[p][k].st = "retired"
MayUpdate(p, k) == Reconciled(p, k) /\ ~NeedsManagedMark(p, k)
NeedsUpdate(p, k) == MayUpdate(p, k) /\ Drift(p, k)
NeedsRetiredMark(p, k) ==
    /\ Locked(p, k)
    /\ cfg[k].st = "retired"
    /\ lock[p][k].st = "managed"
    /\ Ks(p, k).s = "synced" \/ (Ks(p, k).s = "todo" /\ ~Drift(p, k))
NeedsCreate(p, k) ==
    /\ cfg[k].st = "managed"
    /\ ~Locked(p, k)
    /\ k \in Targets => ~Locked(p, MovedFrom[k]) /\ ~Intent(p, MovedFrom[k])

Settled(p, k) ==
    \/ Ks(p, k).s \in {"unknown", "unresolved", "unapproved"}
    \/ Ks(p, k).s \in {"rejected", "deferred", "failed"} /\ ~Intent(p, k)
    \/ /\ Ks(p, k).s = "synced"
       /\ ~NeedsRetiredMark(p, k)
    \/ /\ Ks(p, k).s = "todo"
       /\ ~Intent(p, k)
       /\ ~NeedsMove(p, k) /\ ~NeedsUntrack(p, k) /\ ~NeedsManagedMark(p, k)
       /\ ~NeedsUpdate(p, k) /\ ~NeedsRetiredMark(p, k) /\ ~NeedsCreate(p, k)
Unfinished(p) == {k \in Keys : Ks(p, k).s \notin {"todo", "synced"}}

-----------------------------------------------------------------------------

Creates(p) ==
    {k \in Keys :
        /\ NeedsCreate(p, k)
        /\ ~Intent(p, k)
        /\ Marked(p, k) = {}
        /\ Named(p, k) = {}}

GhostOwnOrphanShown(p) ==
    \E k \in Creates(p), o \in run[p].obs :
        o.kind = Kind[k] /\ o.id \notin LockedIds(p, o.kind) /\ owner[o.kind][o.id] \in Lineage(k)

PlanOf(p, e) ==
    [env |-> e, cfg |-> cfg, lock |-> lock[p],
     ids |-> {<<o.kind, o.id>> : o \in run[p].obs},
     creates |-> {<<Kind[k], k, cfg[k].name>> : k \in Creates(p)},
     ownOrphan |->GhostOwnOrphanShown(p)]

PlanMatches(P, p) ==
    /\ P.env = Env
    /\ P.cfg = cfg
    /\ P.lock = lock[p]
    /\ P.ids = {<<o.kind, o.id>> : o \in run[p].obs}
    /\ P.creates = {<<Kind[k], k, cfg[k].name>> : k \in Creates(p)}

-----------------------------------------------------------------------------

Room(x) == nextId[x] <= MaxId[x]
Paid(x) == x = "badge" /\ freeBadges = 0
CanCommit(r) == Room(r.kind) /\ (~Paid(r.kind) \/ BadgeCost >= 100)
Commit(r) ==
    /\ remote' = [remote EXCEPT ![r.kind][nextId[r.kind]] =
                      [ex |-> TRUE, name |-> r.name, mark |-> r.mark, on |-> TRUE, vis |-> ~ListLag]]
    /\ owner' = [owner EXCEPT ![r.kind][nextId[r.kind]] = r.mark.key]
    /\ nextId' = [nextId EXCEPT ![r.kind] = @ + 1]
    /\ freeBadges' = IF r.kind = "badge" /\ ~Paid(r.kind) THEN freeBadges - 1 ELSE freeBadges
    /\ robux' = IF Paid(r.kind) THEN robux + BadgeCost ELSE robux

Claimed(k) == \E x \in Kinds, i \in Ids : remote[x][i].ex /\ owner[x][i] = k
InFlight(k) == \E r \in pending : r.mark.key = k
UsedNonces(k) ==
    {remote[c[1]][c[2]].mark.n : c \in {c \in Kinds \X Ids : remote[c[1]][c[2]].mark.key = k}}
    \cup {r.mark.n : r \in {q \in pending : q.mark.key = k}}
    \cup {journal[q][k] : q \in Provs}

-----------------------------------------------------------------------------

Unbounded == -1
Budget(max, n) == max = Unbounded \/ n < max
Spend(max, n) == IF max = Unbounded THEN n ELSE n + 1
Fault == Budget(MaxFaults, faults) /\ faults' = Spend(MaxFaults, faults)
SetKs(p, k, v) == run' = [run EXCEPT ![p].ks[k] = v]
Stop(p) == run' = [run EXCEPT ![p] = IdleRun]
Adopted(i) == [id |-> i, st |-> "managed"]
Claim(x, i, k) == [owner EXCEPT ![x][i] = IF @ \in {""} \cup Lineage(k) THEN k ELSE @]

GhostMatchShown(p, k) ==
    \E o \in run[p].obs :
        /\ o.kind = Kind[k]
        /\ \A j \in Keys : Kind[j] = o.kind => lock[p][j].id # o.id
        /\ o.mark.key \in Lineage(k) \/ (o.mark = NoMark /\ o.name = cfg[k].name)

-----------------------------------------------------------------------------

Modes == {"tty"} \cup IF PlanFiles THEN {"file", "plan"} ELSE {}

Start(p, mode) ==
    /\ run[p].pc = "idle"
    /\ mode \in Modes
    /\ Valid(p)
    /\ mode = "file" => planFile[p] # NoPlan /\ (Careful => ~planFile[p].ownOrphan)
    /\ run' = [run EXCEPT ![p].pc = "listing", ![p].mode = mode]
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, roblox, ghosts, faults, edits>>

ListPage(p, x) ==
    /\ run[p].pc = "listing"
    /\ run[p].pages[x] < PageCount
    /\ \A y \in Kinds : Rank(y) < Rank(x) => run[p].pages[y] = PageCount
    /\ SettleFirst => pending = {}
    /\ LET j == run[p].pages[x] + 1
           shown == {[kind |-> x, id |-> i, name |-> remote[x][i].name,
                      mark |-> remote[x][i].mark, on |-> remote[x][i].on] :
                         i \in {i \in Page(x, j) : remote[x][i].ex /\ remote[x][i].vis}}
       IN \/ /\ run' = [run EXCEPT ![p].pages[x] = j, ![p].obs = @ \cup shown]
             /\ UNCHANGED faults
          \/ /\ Fault
             /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, roblox, ghosts, edits>>

Approve(p, yes) ==
    /\ run[p].pc = "listing"
    /\ run[p].mode = "tty"
    /\ ListingDone(p)
    /\ yes /\ Careful => ~GhostOwnOrphanShown(p)
    /\ run' = [run EXCEPT ![p].pc = "working", ![p].ok = IF yes THEN Creates(p) ELSE {}]
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, roblox, ghosts, faults, edits>>

UsePlan(p) ==
    /\ run[p].pc = "listing"
    /\ run[p].mode = "file"
    /\ ListingDone(p)
    /\ IF PlanMatches(planFile[p], p)
       THEN run' = [run EXCEPT ![p].pc = "working",
                               ![p].ok = {c[2] : c \in planFile[p].creates}]
       ELSE Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, roblox, ghosts, faults, edits>>

WritePlan(p, e) ==
    /\ run[p].pc = "listing"
    /\ run[p].mode = "plan"
    /\ ListingDone(p)
    /\ planFile' = [planFile EXCEPT ![p] = PlanOf(p, e)]
    /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, roblox, ghosts, faults, edits>>

ResolveIntent(p, k) ==
    /\ Working(p)
    /\ StaleIntent(p, k)
    /\ ~Locked(p, k)
    /\ LET exact == {o \in Marked(p, k) : o.mark = Mark(k, journal[p][k])}
           other == Marked(p, k) \ exact
       IN \/ /\ other # {}
             /\ Stop(p)
             /\ UNCHANGED <<lock, owner>>
          \/ /\ other = {}
             /\ exact # {}
             /\ LET i == (CHOOSE o \in exact : TRUE).id
                IN /\ lock' = [lock EXCEPT ![p][k] = Adopted(i)]
                   /\ owner' = Claim(Kind[k], i, k)
             /\ SetKs(p, k, [s |-> "adopted", id |-> 0])
          \/ /\ other = {}
             /\ exact = {}
             /\ SetKs(p, k, [s |-> "unresolved", id |-> 0])
             /\ UNCHANGED <<lock, owner>>
    /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, blind, shadowed, faults, edits>>

ClearIntent(p, k) ==
    /\ Working(p)
    /\ Intent(p, k)
    /\ \/ Ks(p, k).s \in {"adopted", "locked", "rejected", "deferred"}
       \/ Ks(p, k).s = "todo" /\ Locked(p, k)
    /\ journal' = [journal EXCEPT ![p][k] = NoIntent]
    /\ SetKs(p, k, IF Ks(p, k).s \in {"rejected", "deferred"} THEN Ks(p, k) ELSE Fresh)
    /\ UNCHANGED <<cfg, lock, committed, planFile, roblox, ghosts, faults, edits>>

Move(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsMove(p, k)
    /\ LET a == MovedFrom[k]
       IN /\ lock' = [lock EXCEPT ![p][k] = lock[p][a], ![p][a] = NoEntry]
          /\ owner' = [owner EXCEPT ![Kind[k]][lock[p][a].id] = IF @ = a THEN k ELSE @]
    /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, run, blind, shadowed,
                   faults, edits>>

Untrack(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsUntrack(p, k)
    /\ lock' = [lock EXCEPT ![p][k].st = "untracked"]
    /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, run, ghosts, faults, edits>>

WriteLockManaged(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsManagedMark(p, k)
    /\ lock' = [lock EXCEPT ![p][k].st = "managed"]
    /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, run, ghosts, faults, edits>>

Payload(p, k) ==
    [kind |-> Kind[k], id |-> lock[p][k].id, declared |-> Declared(k),
     name |-> cfg[k].name, on |-> DesiredOn(k),
     why |-> cfg[k].st = "retired" \/ (cfg[k].st = "absent" /\ lock[p][k].st = "retired")]
Apply(r) ==
    [remote EXCEPT ![r.kind][r.id].on = r.on,
                   ![r.kind][r.id].name = IF r.declared THEN r.name ELSE @,
                   ![r.kind][r.id].mark = IF r.declared THEN NoMark ELSE @]

Update(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ MayUpdate(p, k)
    /\ LET r == Payload(p, k)
       IN \/ /\ remote' = Apply(r)
             /\ SetKs(p, k, [s |-> "synced", id |-> 0])
             /\ UNCHANGED <<late, faults>>
          \/ /\ Fault
             /\ \/ remote' = Apply(r) /\ UNCHANGED late
                \/ UNCHANGED <<remote, late>>
                \/ LateUpdates /\ late' = late \cup {r} /\ UNCHANGED remote
             /\ SetKs(p, k, [s |-> "failed", id |-> 0])
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, nextId, pending, freeBadges, robux,
                   ghosts, edits>>

WriteLockRetired(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ NeedsRetiredMark(p, k)
    /\ lock' = [lock EXCEPT ![p][k].st = "retired"]
    /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, run, ghosts, faults, edits>>

AdoptCheck(p, k) ==
    /\ Working(p)
    /\ NoStaleIntents(p)
    /\ Ks(p, k).s = "todo"
    /\ NeedsCreate(p, k)
    /\ LET M == IF Marked(p, k) # {} THEN Marked(p, k) ELSE Named(p, k)
       IN \/ /\ Cardinality(M) = 1
             /\ LET i == (CHOOSE o \in M : TRUE).id
                IN /\ lock' = [lock EXCEPT ![p][k] = Adopted(i)]
                   /\ owner' = Claim(Kind[k], i, k)
             /\ UNCHANGED run
          \/ /\ M = {}
             /\ SetKs(p, k, [s |-> IF k \in run[p].ok THEN "clear" ELSE "unapproved", id |-> 0])
             /\ UNCHANGED <<lock, owner>>
          \/ /\ Cardinality(M) > 1
             /\ Stop(p)
             /\ UNCHANGED <<lock, owner>>
    /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, blind, shadowed, faults, edits>>

WriteIntent(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "clear"
    /\ \E n \in Nonces \ UsedNonces(k) : journal' = [journal EXCEPT ![p][k] = n]
    /\ SetKs(p, k, [s |-> "intent", id |-> 0])
    /\ UNCHANGED <<cfg, lock, committed, planFile, roblox, ghosts, faults, edits>>

Create(p, k) ==
    LET x == Kind[k]
        req == [kind |-> x, name |-> cfg[k].name, mark |-> Mark(k, journal[p][k])]
    IN
    /\ Working(p)
    /\ Ks(p, k).s = "intent"
    /\ blind' = (blind \/ \E j \in Lineage(k) : InFlight(j) \/ Claimed(j))
    /\ shadowed' = (shadowed \/ GhostMatchShown(p, k))
    /\ \/ /\ CanCommit(req)
          /\ Commit(req)
          /\ SetKs(p, k, [s |-> "created", id |-> nextId[x]])
          /\ UNCHANGED <<pending, late, faults>>
       \/ /\ Fault
          /\ pending' = pending \cup {req}
          /\ SetKs(p, k, [s |-> "unknown", id |-> 0])
          /\ UNCHANGED <<remote, owner, nextId, late, freeBadges, robux>>
       \/ /\ Fault
          /\ SetKs(p, k, [s |-> "rejected", id |-> 0])
          /\ UNCHANGED <<roblox, owner>>
       \/ /\ ~CanCommit(req)
          /\ SetKs(p, k, [s |-> IF Room(x) THEN "deferred" ELSE "rejected", id |-> 0])
          /\ UNCHANGED <<roblox, owner, faults>>
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, edits>>

WriteLockCreated(p, k) ==
    /\ Working(p)
    /\ Ks(p, k).s = "created"
    /\ lock' = [lock EXCEPT ![p][k] = [id |-> Ks(p, k).id, st |-> "managed"]]
    /\ SetKs(p, k, [s |-> "locked", id |-> 0])
    /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, ghosts, faults, edits>>

Finish(p) ==
    /\ Working(p)
    /\ \A k \in Keys : Settled(p, k)
    /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, roblox, ghosts, faults, edits>>

Crash(p) ==
    /\ run[p].pc # "idle"
    /\ Fault
    /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, roblox, ghosts, edits>>

KeyStep(p, k) ==
    \/ ResolveIntent(p, k) \/ ClearIntent(p, k) \/ Move(p, k) \/ Untrack(p, k)
    \/ WriteLockManaged(p, k) \/ Update(p, k) \/ WriteLockRetired(p, k)
    \/ AdoptCheck(p, k) \/ WriteIntent(p, k) \/ Create(p, k) \/ WriteLockCreated(p, k)

Provision(p) ==
    \/ Finish(p)
    \/ \E mode \in Modes : Start(p, mode)
    \/ \E x \in Kinds : ListPage(p, x)
    \/ \E yes \in BOOLEAN : Approve(p, yes)
    \/ UsePlan(p)
    \/ \E e \in Envs : WritePlan(p, e)
    \/ \E k \in Keys : KeyStep(p, k)

-----------------------------------------------------------------------------

AdoptRead(p, k, i) ==
    LET x == Kind[k]
    IN /\ run[p].pc = "idle"
       /\ Declared(k)
       /\ ~Locked(p, k)
       /\ remote[x][i].ex
       /\ i \notin LockedIds(p, x)
       /\ \/ owner[x][i] \in Lineage(k)
          \/ owner[x][i] = "" /\ \A j \in Lineage(k) : ~InFlight(j) /\ ~Claimed(j)
       /\ run' = [run EXCEPT ![p].pc = "adopting", ![p].mode = "adopt",
                             ![p].op = [k |-> k, i |-> i]]
       /\ UNCHANGED <<cfg, lock, journal, committed, planFile, roblox, ghosts, faults, edits>>

AdoptLock(p) ==
    LET k == run[p].op.k
    IN /\ run[p].pc = "adopting"
       /\ lock' = [lock EXCEPT ![p][k] = Adopted(run[p].op.i)]
       /\ owner' = Claim(Kind[k], run[p].op.i, k)
       /\ run' = [run EXCEPT ![p].pc = "adopted"]
       /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, blind, shadowed, faults, edits>>

AdoptClear(p) ==
    /\ run[p].pc = "adopted"
    /\ journal' = [journal EXCEPT ![p][run[p].op.k] = NoIntent]
    /\ Stop(p)
    /\ UNCHANGED <<cfg, lock, committed, planFile, roblox, ghosts, faults, edits>>

OperatorNotCreated(p, k) ==
    /\ run[p].pc = "idle"
    /\ Intent(p, k)
    /\ ~Locked(p, k)
    /\ ~InFlight(k) /\ ~Claimed(k)
    /\ journal' = [journal EXCEPT ![p][k] = NoIntent]
    /\ UNCHANGED <<cfg, lock, committed, planFile, roblox, run, ghosts, faults, edits>>

Operate(p) ==
    \/ \E k \in Keys : OperatorNotCreated(p, k)
    \/ \E k \in Keys, i \in Ids : AdoptRead(p, k, i)
    \/ AdoptLock(p) \/ AdoptClear(p)

Idle == \A p \in Provs : run[p].pc = "idle"

Edit(k, st, name) ==
    /\ Idle
    /\ Budget(MaxEdits, edits)
    /\ (st \in {"absent", "untrack"}) = (name = NoName)
    /\ [st |-> st, name |-> name] # cfg[k]
    /\ st \in {"managed", "retired"} =>
           \A j \in Keys \ {k} : Kind[j] = Kind[k] /\ Declared(j) => cfg[j].name # name
    /\ cfg' = [cfg EXCEPT ![k] = [st |-> st, name |-> name]]
    /\ edits' = Spend(MaxEdits, edits)
    /\ UNCHANGED <<lock, journal, committed, planFile, roblox, run, ghosts, faults>>

CommitLock(p) ==
    /\ "checkout" \in LoseLocal
    /\ run[p].pc = "idle"
    /\ committed[p] # lock[p]
    /\ committed' = [committed EXCEPT ![p] = lock[p]]
    /\ UNCHANGED <<cfg, lock, journal, planFile, roblox, run, ghosts, faults, edits>>

DiscardCheckout(p) ==
    /\ "checkout" \in LoseLocal
    /\ run[p].pc = "idle"
    /\ lock[p] # committed[p]
    /\ Fault
    /\ lock' = [lock EXCEPT ![p] = committed[p]]
    /\ UNCHANGED <<cfg, journal, committed, planFile, roblox, run, ghosts, edits>>

DiscardHome(p) ==
    /\ "home" \in LoseLocal
    /\ run[p].pc = "idle"
    /\ \E k \in Keys : Intent(p, k)
    /\ Fault
    /\ journal' = [journal EXCEPT ![p] = [k \in Keys |-> NoIntent]]
    /\ UNCHANGED <<cfg, lock, committed, planFile, roblox, run, ghosts, edits>>

NewDay ==
    /\ freeBadges < FreeQuota
    /\ freeBadges' = FreeQuota
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, remote, nextId, pending, late, robux,
                   run, ghosts, faults, edits>>

Propagate(x, i) ==
    /\ ListLag
    /\ remote[x][i].ex
    /\ ~remote[x][i].vis
    /\ remote' = [remote EXCEPT ![x][i].vis = TRUE]
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, nextId, pending, late, freeBadges,
                   robux, run, ghosts, faults, edits>>

Settle(r) ==
    /\ r \in pending
    /\ pending' = pending \ {r}
    /\ \/ CanCommit(r) /\ Commit(r)
       \/ UNCHANGED <<remote, owner, nextId, freeBadges, robux>>
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, late, run, blind, shadowed,
                   faults, edits>>

SettleUpdate(r) ==
    /\ r \in late
    /\ late' = late \ {r}
    /\ remote' \in {remote, Apply(r)}
    /\ UNCHANGED <<cfg, lock, journal, committed, planFile, nextId, pending, freeBadges, robux,
                   run, ghosts, faults, edits>>

-----------------------------------------------------------------------------

Init ==
    /\ cfg = [k \in Keys |-> IF k \in Targets THEN [st |-> "absent", name |-> NoName]
                                               ELSE [st |-> "managed", name |-> Name0[k]]]
    /\ lock = [p \in Provs |-> [k \in Keys |-> NoEntry]]
    /\ journal = [p \in Provs |-> [k \in Keys |-> NoIntent]]
    /\ committed = lock
    /\ planFile = [p \in Provs |-> NoPlan]
    /\ \E count \in [Kinds -> 0..2], hn \in [Kinds -> [Ids -> Names]],
          hv \in [Kinds -> [Ids -> IF ListLag THEN BOOLEAN ELSE {TRUE}]] :
           /\ \A x \in Kinds : count[x] <= Handmade[x]
           /\ remote = [x \in Kinds |-> [i \in Ids |->
                           IF i <= count[x]
                           THEN [ex |-> TRUE, name |-> hn[x][i], mark |-> NoMark, on |-> TRUE,
                                 vis |-> hv[x][i]]
                           ELSE NoRec]]
           /\ nextId = [x \in Kinds |-> count[x] + 1]
    /\ pending = {}
    /\ late = {}
    /\ freeBadges \in 0..FreeQuota
    /\ robux = 0
    /\ run = [p \in Provs |-> IdleRun]
    /\ owner = [x \in Kinds |-> [i \in Ids |-> ""]]
    /\ blind = FALSE
    /\ shadowed = FALSE
    /\ faults = 0
    /\ edits = 0

Next ==
    \/ \E p \in Provs :
          \/ Provision(p) \/ Crash(p) \/ Operate(p)
          \/ CommitLock(p) \/ DiscardCheckout(p) \/ DiscardHome(p)
    \/ \E k \in Keys, st \in Statuses, name \in Names \cup {NoName} : Edit(k, st, name)
    \/ NewDay
    \/ \E x \in Kinds, i \in Ids : Propagate(x, i)
    \/ \E r \in pending : Settle(r)
    \/ \E r \in late : SettleUpdate(r)

Spec == Init /\ [][Next]_vars

LiveSpec ==
    /\ Spec
    /\ \A p \in Provs :
          /\ WF_vars(Provision(p))
          /\ SF_vars(Approve(p, TRUE))
          /\ WF_vars(AdoptLock(p) \/ AdoptClear(p))
    /\ WF_vars(NewDay)
    /\ WF_vars(\E x \in Kinds, i \in Ids : Propagate(x, i))
    /\ WF_vars(\E r \in pending : Settle(r))
    /\ WF_vars(\E r \in late : SettleUpdate(r))

OperatorSpec == LiveSpec /\ \A p \in Provs : SF_vars(\E k \in Keys : OperatorNotCreated(p, k))

-----------------------------------------------------------------------------

TypeOK ==
    /\ cfg \in [Keys -> [st : Statuses, name : Names \cup {NoName}]]
    /\ \A x \in Kinds : nextId[x] \in 1..(MaxId[x] + 1)
    /\ freeBadges \in 0..FreeQuota
    /\ blind \in BOOLEAN
    /\ \A p \in Provs : run[p].pc \in {"idle", "listing", "working", "adopting", "adopted"}

AtMostOneRemotePerKey ==
    \A k \in Keys :
        Cardinality({<<x, i>> \in Kinds \X Ids : remote[x][i].ex /\ owner[x][i] = k}) <= 1

JournalCoversCreates ==
    \A p \in Provs, k \in Keys :
        (InFlight(k) \/ Claimed(k)) /\ ~Locked(p, k) => Intent(p, k)

LockNamesExisting ==
    \A p \in Provs, k \in Keys : Locked(p, k) => remote[Kind[k]][lock[p][k].id].ex

LockInjective ==
    \A p \in Provs, a, b \in Keys :
        a # b /\ Kind[a] = Kind[b] /\ Locked(p, a) /\ Locked(p, b) => lock[p][a].id # lock[p][b].id

LockOwnsItsResource ==
    \A p \in Provs, k \in Keys : Locked(p, k) => owner[Kind[k]][lock[p][k].id] = k

RetiredIsOff ==
    \A p \in Provs, k \in Keys :
        lock[p][k].st = "retired" => ~remote[Kind[k]][lock[p][k].id].on

NoBlindCreate == ~blind

NoCreateBesideMatch == ~shadowed

NoPaidBadge == robux = 0

RemovedIsError == \A p \in Provs : run[p].pc \in {"listing", "working"} => Valid(p)

RunAdvances ==
    \A p \in Provs : run[p].pc # "idle" => ENABLED (Provision(p) \/ AdoptLock(p) \/ AdoptClear(p))

NoSilentDelete ==
    [][\A p \in Provs, k \in Keys :
          (Locked(p, k) /\ lock'[p][k] = NoEntry) => MovedAway(k)]_vars

OffAllowed(x, i) ==
    \/ \E p \in Provs, k \in Keys :
           /\ Kind[k] = x
           /\ lock[p][k].id = i
           /\ cfg[k].st = "retired" \/ (cfg[k].st = "absent" /\ lock[p][k].st = "retired")
    \/ \E r \in late \ late' : r.kind = x /\ r.id = i /\ r.why

NoSilentRetire ==
    [][\A x \in Kinds, i \in Ids :
          remote[x][i].ex /\ remote[x][i].on /\ ~remote'[x][i].on => OffAllowed(x, i)]_vars

MovedKeepsId ==
    [][\A p \in Provs, t \in Targets :
          (Locked(p, MovedFrom[t]) /\ lock'[p][MovedFrom[t]].id = 0 /\ cfg[t].st # "absent") =>
              lock'[p][t].id = lock[p][MovedFrom[t]].id]_vars

PlanIsFresh ==
    [][\A p \in Provs :
          run[p].mode = "file" /\ run[p].pc = "listing" /\ run'[p].pc = "working" =>
              [planFile[p] EXCEPT !.ownOrphan = FALSE] = [PlanOf(p, Env) EXCEPT !.ownOrphan = FALSE]]_vars

Writes(a, b) == IF a = b THEN 0 ELSE 1

OneWritePerStep ==
    [][Writes(lock, lock') + Writes(journal, journal') + Writes(planFile, planFile')
           + Writes(roblox, roblox') <= 1]_vars

KeyConverged(p, k) ==
    /\ ~Intent(p, k)
    /\ cfg[k].st = "managed" => Locked(p, k)
    /\ Reconciled(p, k) =>
           LET r == remote[Kind[k]][lock[p][k].id]
           IN /\ r.on = DesiredOn(k)
              /\ lock[p][k].st = IF cfg[k].st = "managed" THEN "managed" ELSE "retired"
              /\ Declared(k) => r.name = cfg[k].name /\ r.mark = NoMark
    /\ cfg[k].st = "untrack" => lock[p][k].st \in {"none", "untracked"}
    /\ cfg[k].st = "absent" => lock[p][k].st # "managed"

NeedsOperator(p, k) == Intent(p, k) /\ ~Locked(p, k) /\ ~InFlight(k) /\ ~Claimed(k)

Converges == \A p \in Provs : <>[]((\A k \in Keys : KeyConverged(p, k)) \/ ~Valid(p))

ConvergesOrAsks ==
    \A p \in Provs, k \in Keys : <>[](KeyConverged(p, k) \/ NeedsOperator(p, k) \/ ~Valid(p))
=============================================================================
