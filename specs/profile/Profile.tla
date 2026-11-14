------------------------------- MODULE Profile -------------------------------
(***************************************************************************)
(* One player's profile under a profile store: the lease each server's     *)
(* session takes in the DataStore record, the local authority a session    *)
(* holds while its safe bound lasts, the owned writes that renew the lease *)
(* and carry the session's data, the release that ends it, and Roblox's    *)
(* ProcessReceipt callbacks that grant developer products into it.         *)
(* packages/crucible/src/internal/data/profiles.ts, session.ts, lease.ts   *)
(* and receipts.ts implement it.                                           *)
(*                                                                         *)
(* Every server reads one clock, `now` (A1). A request lands in the        *)
(* DataStore atomically at some time after it was sent, and its answer can *)
(* be lost after it landed or before. A server sends one request for the   *)
(* player's key at a time (A2), but a load that stops waiting leaves its   *)
(* acquire to land later. specs/profile/README.md states the assumptions   *)
(* and what each property means.                                           *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS
    Servers,      \* the game servers the player can join
    Purchases,    \* the developer product purchases the player made
    Lease,        \* LEASE_SECONDS: how long a write keeps the lease
    Local,        \* LOCAL_AUTHORITY_SECONDS: how long after a write was sent its session stays writable
    Grace,        \* TAKEOVER_GRACE_SECONDS: how long past expiry another server waits to take the lease
    MaxTime,      \* the clock stops here
    MaxSessions,  \* sessions made in total, one token each
    MaxFaults     \* requests whose answer is lost, in total

VARIABLES
    now,     \* the clock
    at,      \* the server Roblox routes the player to, and calls ProcessReceipt on
    here,    \* the servers the player is present on: `at`, and one that has not yet handled PlayerRemoving
    store,   \* the DataStore record: its epoch, lease, commit marker, payload and receipts
    sess,    \* sess[t]: the session with token t
    cur,     \* cur[s]: the token of the session server s holds for the player, or 0
    req,     \* req[s]: server s's one request for the player's key
    cb,      \* cb[s][p]: Roblox's ProcessReceipt callback for purchase p on server s
    acked,   \* purchases Roblox was answered PurchaseGranted for
    ran,     \* ghost: ran[p] counts the afterGrant effects run for p
    faults,  \* lost answers so far
    next     \* the next unused token

vars == <<now, at, here, store, sess, cur, req, cb, acked, ran, faults, next>>

Tokens == 1..MaxSessions
First == CHOOSE s \in Servers : TRUE
Second == CHOOSE s \in Servers : s # First
Max(a, b) == IF a > b THEN a ELSE b

\* The payload is what the grants did: payload[p] counts the grants of p it holds.
NoGrants == [p \in Purchases |-> 0]

\* A session's data as an owned write or a carrying acquire stores it.
Snap(x) == [value |-> x.value, rev |-> x.rev, staged |-> x.staged]

Fresh(s) == [srv |-> s, phase |-> "loading", by |-> "none", releasing |-> FALSE,
             released |-> "none", epoch |-> 0, safe |-> 0, earliest |-> -1, value |-> NoGrants, rev |-> 0,
             staged |-> {}, durable |-> {}, effects |-> {}, writing |-> FALSE, op |-> 0,
             want |-> FALSE, kept |-> FALSE, owes |-> FALSE]

Unused == [Fresh(CHOOSE s \in Servers : TRUE) EXCEPT !.phase = "unused"]

Absent == [epoch |-> 0, lease |-> 0, exp |-> 0, ctok |-> 0, crev |-> 0, cop |-> 0, payload |-> NoGrants,
           receipts |-> {}]

\* What an answer carries back: the envelope's epoch, payload and receipts.
Env(r) == [epoch |-> r.epoch, payload |-> r.payload, receipts |-> r.receipts]

NoReq == [kind |-> "none", tok |-> 0, op |-> 0, rel |-> FALSE, sent |-> 0, first |-> -1, deadline |-> 0,
          epoch |-> 0, snap |-> Snap(Unused), sup |-> {}, st |-> "none", dec |-> "none", env |-> Env(Absent)]

NoCall == [st |-> "idle", tok |-> 0]

\* Rule 9: writable exactly while active and before the safe bound.
Writable(t) == sess[t].phase = "active" /\ now < sess[t].safe

\* Rule 26: a seal stops local writes at once and asks for the release.
Seal(x, why) ==
    IF x.phase = "active"
    THEN [x EXCEPT !.phase = "sealed", !.by = why, !.releasing = TRUE, !.kept = TRUE, !.want = FALSE]
    ELSE IF x.phase = "sealed" THEN [x EXCEPT !.releasing = TRUE] ELSE x

\* The release's outcome ends the session (rule 29).
Released(x, outcome) == [x EXCEPT !.phase = "ended", !.released = outcome, !.writing = FALSE, !.safe = 0, !.op = 0]

\* A load that stops ends its session and owes a write that removes its own lease (rule 20).
EndLoad(x) == [x EXCEPT !.phase = "ended", !.owes = TRUE, !.earliest = -1]

\* A server sends one request for the key at a time, and a new session's acquire waits for the sealed
\* session's release and any abandoned load's lease removal (rules 29, 30, 47).
LaneFree(s) == req[s].kind = "none"
Settled(s) == \A u \in Tokens : sess[u].srv = s =>
                  ~sess[u].owes /\ ~(sess[u].phase = "sealed" /\ sess[u].released = "none")

\* Each tick makes every active session's renewal due (armRenewal).
Tick ==
    /\ now < MaxTime
    /\ now' = now + 1
    /\ sess' = [t \in Tokens |-> IF sess[t].phase = "active" THEN [sess[t] EXCEPT !.want = TRUE] ELSE sess[t]]
    /\ UNCHANGED <<at, here, store, cur, req, cb, acked, ran, faults, next>>

\* The player teleports to the other server, once. The first server sees PlayerRemoving later.
Move ==
    /\ at = First
    /\ at' = Second
    /\ here' = here \cup {Second}
    /\ UNCHANGED <<now, store, sess, cur, req, cb, acked, ran, faults, next>>

\* PlayerRemoving: rule 43's release. A loading session is abandoned, and the wake ends its load at once; an
\* active one seals.
Leave(s) ==
    /\ s \in here
    /\ s # at
    /\ here' = here \ {s}
    /\ sess' = IF cur[s] = 0 THEN sess
               ELSE IF sess[cur[s]].phase = "loading" THEN [sess EXCEPT ![cur[s]] = EndLoad(@)]
               ELSE [sess EXCEPT ![cur[s]] = Seal(@, "leave")]
    /\ UNCHANGED <<now, at, store, cur, req, cb, acked, ran, faults, next>>

\* A join or a resume loads a new session unless one is loading or active.
StartLoad(s) ==
    /\ s \in here
    /\ next <= MaxSessions
    /\ IF cur[s] = 0 THEN TRUE ELSE sess[cur[s]].phase \in {"sealed", "ended"}
    /\ sess' = [sess EXCEPT ![next] = Fresh(s)]
    /\ cur' = [cur EXCEPT ![s] = next]
    /\ next' = next + 1
    /\ UNCHANGED <<now, at, here, store, req, cb, acked, ran, faults>>

\* This server's sealed sessions whose release settled, which an acquire may supersede (rule 29).
Supersedes(s) == {[tok |-> u, epoch |-> sess[u].epoch, snap |-> Snap(sess[u])] :
                    u \in {u \in Tokens : sess[u].srv = s /\ sess[u].kept /\ sess[u].released # "none"}}

SendAcquire(t) ==
    LET s == sess[t].srv IN
    /\ sess[t].phase = "loading"
    /\ s \in here
    /\ LaneFree(s)
    /\ Settled(s)
    /\ req' = [req EXCEPT ![s] = [NoReq EXCEPT !.kind = "acq", !.tok = t, !.sent = now, !.sup = Supersedes(s),
                                               !.st = "flight"]]
    /\ UNCHANGED <<now, at, here, store, sess, cur, cb, acked, ran, faults, next>>

\* Rule 24: the lease is taken when absent, past expiry plus grace, or our own sealed session's.
AcquireDecision(r) ==
    LET found == {x \in r.sup : x.tok = store.lease} IN
    IF store.lease = r.tok THEN "already"
    ELSE IF store.lease # 0 /\ found = {} /\ now <= store.exp + Grace THEN "held"
    ELSE "taken"

Taken(r) ==
    LET l == store.lease
        found == {x \in r.sup : x.tok = l}
        x == CHOOSE y \in found : TRUE
        base == [store EXCEPT !.epoch = @ + 1, !.lease = r.tok, !.exp = now + Lease]
    IN IF found # {} /\ store.epoch = x.epoch /\ (store.ctok # l \/ store.crev < x.snap.rev)
       THEN [base EXCEPT !.payload = x.snap.value, !.receipts = @ \cup x.snap.staged, !.ctok = l,
                         !.crev = x.snap.rev, !.cop = IF store.ctok = l THEN store.cop ELSE 0]
       ELSE base

LandAcquire(s) ==
    LET r == req[s]
        d == AcquireDecision(r)
        after == IF d = "taken" THEN Taken(r) ELSE store
    IN /\ r.kind = "acq"
       /\ r.st = "flight"
       /\ store' = after
       /\ req' = [req EXCEPT ![s].st = "landed", ![s].dec = d, ![s].env = Env(after)]
       /\ UNCHANGED <<now, at, here, sess, cur, cb, acked, ran, faults, next>>

\* Rules 17 to 21: a session becomes writable from the earliest send of a write that may have taken the lease.
Activate(x, env, safe) ==
    [x EXCEPT !.phase = "active", !.epoch = env.epoch, !.safe = safe, !.earliest = -1, !.value = env.payload,
              !.rev = 0, !.staged = env.receipts, !.durable = env.receipts]

ReplyAcquire(s) ==
    LET r == req[s]
        t == r.tok
        x == sess[t]
        earliest == IF x.earliest = -1 THEN r.sent ELSE x.earliest
    IN /\ r.kind = "acq"
       /\ r.st = "landed"
       /\ req' = [req EXCEPT ![s] = NoReq]
       /\ sess' =
            IF x.phase # "loading" THEN sess
            ELSE IF r.dec = "held" THEN [sess EXCEPT ![t].earliest = -1]
            ELSE IF now >= earliest + Local THEN [sess EXCEPT ![t] = EndLoad(@)]
            ELSE [u \in Tokens |->
                    IF u = t THEN Activate(x, r.env, earliest + Local)
                    ELSE IF sess[u].srv = s /\ sess[u].released # "none" THEN [sess[u] EXCEPT !.kept = FALSE]
                    ELSE sess[u]]
       /\ UNCHANGED <<now, at, here, store, cur, cb, acked, ran, faults, next>>

\* An acquire's answer is lost, whether or not it landed. The load sends another with the same token.
FaultAcquire(s) ==
    LET r == req[s] IN
    /\ r.kind = "acq"
    /\ faults < MaxFaults
    /\ faults' = faults + 1
    /\ req' = [req EXCEPT ![s] = NoReq]
    /\ sess' = IF sess[r.tok].phase = "loading" /\ sess[r.tok].earliest = -1
               THEN [sess EXCEPT ![r.tok].earliest = r.sent] ELSE sess
    /\ UNCHANGED <<now, at, here, store, cur, cb, acked, ran, next>>

\* Rule 20: removes exactly the abandoned load's own lease.
Abandon(t) ==
    LET s == sess[t].srv IN
    /\ sess[t].owes
    /\ LaneFree(s)
    /\ sess' = [sess EXCEPT ![t].owes = FALSE]
    /\ store' = IF store.lease = t THEN [store EXCEPT !.lease = 0, !.exp = 0] ELSE store
    /\ UNCHANGED <<now, at, here, cur, req, cb, acked, ran, faults, next>>

\* Rules 28, 31, 32: one owned write at a time, sent when a renewal or a save is wanted; a sealed session's is
\* its release.
Pump(t) ==
    LET x == sess[t]
        s == x.srv
        rel == x.phase = "sealed" /\ x.releasing /\ x.released = "none"
        deadline == IF rel THEN x.safe - Local + Lease ELSE x.safe
    IN /\ ~x.writing
       /\ rel \/ (x.phase = "active" /\ x.want)
       /\ LaneFree(s)
       /\ IF now < deadline
          THEN /\ req' = [req EXCEPT ![s] = [NoReq EXCEPT !.kind = "own", !.tok = t, !.op = x.op + 1, !.rel = rel,
                                                          !.sent = now, !.deadline = deadline, !.epoch = x.epoch,
                                                          !.snap = Snap(x), !.st = "flight"]]
               /\ sess' = [sess EXCEPT ![t].op = @ + 1, ![t].writing = TRUE, ![t].want = FALSE]
          ELSE /\ rel
               /\ req' = req
               /\ sess' = [sess EXCEPT ![t] = Released(@, "given up")]
       /\ UNCHANGED <<now, at, here, store, cur, cb, acked, ran, faults, next>>

\* Rules 27, 28: fenced on token and epoch, identified by op.
OwnedDecision(r) ==
    IF store.epoch = 0 THEN "fenced"
    ELSE IF store.ctok = r.tok /\ store.cop >= r.op THEN "already"
    ELSE IF store.lease # r.tok \/ store.epoch # r.epoch THEN "fenced"
    ELSE IF now >= store.exp THEN "expired"
    ELSE "written"

Written(r) ==
    LET revision == IF store.ctok = r.tok THEN store.crev ELSE -1
        newer == r.snap.rev > revision
    IN [store EXCEPT !.payload = IF newer THEN r.snap.value ELSE @,
                     !.receipts = IF newer THEN @ \cup r.snap.staged ELSE @,
                     !.lease = IF r.rel THEN 0 ELSE @,
                     !.exp = IF r.rel THEN 0 ELSE now + Lease,
                     !.ctok = r.tok, !.crev = Max(r.snap.rev, revision), !.cop = r.op]

LandOwned(s) ==
    LET r == req[s]
        d == OwnedDecision(r)
        after == IF d = "written" THEN Written(r) ELSE store
    IN /\ r.kind = "own"
       /\ r.st = "flight"
       /\ store' = after
       /\ req' = [req EXCEPT ![s].st = "landed", ![s].dec = d, ![s].env = Env(after)]
       /\ UNCHANGED <<now, at, here, sess, cur, cb, acked, ran, faults, next>>

\* Rule 28: a confirmed write is exactly what the stored envelope holds, for this token only. It extends the
\* safe bound from the earliest send of the operation that may have stored it.
Confirm(x, r) ==
    LET earliest == IF r.first = -1 THEN r.sent ELSE r.first
        y == [x EXCEPT !.durable = r.env.receipts]
    IN IF y.phase # "active" THEN y
       ELSE IF now >= y.safe THEN Seal(y, "authority")
       ELSE [y EXCEPT !.safe = Max(@, earliest + Local)]

ReplyOwned(s) ==
    LET r == req[s]
        t == r.tok
        x == [sess[t] EXCEPT !.writing = FALSE]
        committed == r.dec \in {"written", "already"}
        y == IF committed THEN Confirm(x, r) ELSE x
    IN /\ r.kind = "own"
       /\ r.st = "landed"
       /\ req' = [req EXCEPT ![s] = NoReq]
       /\ sess' = [sess EXCEPT ![t] =
            IF r.rel THEN Released(y, IF committed THEN "committed" ELSE "fenced")
            ELSE IF committed THEN y
            ELSE Seal(y, IF r.dec = "fenced" THEN "fenced" ELSE "authority")]
       /\ UNCHANGED <<now, at, here, store, cur, cb, acked, ran, faults, next>>

\* An owned write's answer is lost, whether or not it landed. The scheduler sends it again with the session's
\* newest snapshot before its deadline, or gives it up: a release that gives up has not committed.
FaultOwned(s) ==
    LET r == req[s]
        t == r.tok
        first == IF r.first = -1 THEN r.sent ELSE r.first
    IN /\ r.kind = "own"
       /\ faults < MaxFaults
       /\ faults' = faults + 1
       /\ \/ /\ now < r.deadline
             /\ req' = [req EXCEPT ![s] = [r EXCEPT !.first = first, !.sent = now, !.snap = Snap(sess[t]),
                                                    !.st = "flight", !.dec = "none"]]
             /\ sess' = sess
          \/ /\ req' = [req EXCEPT ![s] = NoReq]
             /\ sess' = [sess EXCEPT ![t] = IF r.rel THEN Released(@, "given up") ELSE [@ EXCEPT !.writing = FALSE]]
       /\ UNCHANGED <<now, at, here, store, cur, cb, acked, ran, next>>

\* armSeal: a passed safe bound seals the session and starts its release.
SafeBound(t) ==
    /\ sess[t].phase = "active"
    /\ now >= sess[t].safe
    /\ sess' = [sess EXCEPT ![t] = Seal(@, "authority")]
    /\ UNCHANGED <<now, at, here, store, cur, req, cb, acked, ran, faults, next>>

\* Rule 61: PurchaseGranted runs the effects this session staged with its grant, once.
Ack(t, p) ==
    /\ acked' = acked \cup {p}
    /\ IF p \in sess[t].effects
       THEN /\ ran' = [ran EXCEPT ![p] = @ + 1]
            /\ sess' = [sess EXCEPT ![t].effects = @ \ {p}]
       ELSE UNCHANGED <<ran, sess>>

\* Rules 58 to 60: ProcessReceipt for p on server s. It holds while the session loads, refuses without
\* authority, acknowledges a durable id, grants a new one in one revision with its id, and waits for a staged one.
Process(s, p) ==
    LET t == cur[s] IN
    IF s \notin here THEN /\ cb' = [cb EXCEPT ![s][p] = NoCall] /\ UNCHANGED <<sess, acked, ran>>
    ELSE IF t = 0 THEN /\ cb' = [cb EXCEPT ![s][p] = NoCall] /\ UNCHANGED <<sess, acked, ran>>
    ELSE IF ~Writable(t) THEN /\ cb' = [cb EXCEPT ![s][p] = NoCall] /\ UNCHANGED <<sess, acked, ran>>
    ELSE IF p \in sess[t].durable THEN /\ cb' = [cb EXCEPT ![s][p] = NoCall] /\ Ack(t, p)
    ELSE IF p \notin sess[t].staged
         THEN /\ sess' = [sess EXCEPT ![t].value[p] = @ + 1, ![t].rev = @ + 1, ![t].staged = @ \cup {p},
                                      ![t].effects = @ \cup {p}, ![t].want = TRUE]
              /\ cb' = [cb EXCEPT ![s][p] = [st |-> "save", tok |-> t]]
              /\ UNCHANGED <<acked, ran>>
    ELSE /\ sess' = [sess EXCEPT ![t].want = TRUE]
         /\ cb' = [cb EXCEPT ![s][p] = [st |-> "save", tok |-> t]]
         /\ UNCHANGED <<acked, ran>>

\* Roblox calls ProcessReceipt for a purchase it has not been told is granted, where the player is.
Call(s, p) ==
    /\ s = at
    /\ p \notin acked
    /\ cb[s][p].st = "idle"
    /\ Process(s, p)
    /\ UNCHANGED <<now, at, here, store, cur, req, faults, next>>

\* Rule 63: a granting callback answers PurchaseGranted once its id is durable, and NotProcessedYet when its
\* session ends or its wait times out.
Wait(s, p) ==
    LET t == cb[s][p].tok IN
    /\ cb[s][p].st = "save"
    /\ \/ /\ p \in sess[t].durable
          /\ cb' = [cb EXCEPT ![s][p] = NoCall]
          /\ Ack(t, p)
       \/ /\ p \notin sess[t].durable
          /\ cb' = [cb EXCEPT ![s][p] = NoCall]
          /\ UNCHANGED <<sess, acked, ran>>
    /\ UNCHANGED <<now, at, here, store, cur, req, faults, next>>

Init ==
    /\ now = 0
    /\ at = First
    /\ here = {First}
    /\ store = Absent
    /\ sess = [t \in Tokens |-> Unused]
    /\ cur = [s \in Servers |-> 0]
    /\ req = [s \in Servers |-> NoReq]
    /\ cb = [s \in Servers |-> [p \in Purchases |-> NoCall]]
    /\ acked = {}
    /\ ran = [p \in Purchases |-> 0]
    /\ faults = 0
    /\ next = 1

Next ==
    \/ Tick
    \/ Move
    \/ \E s \in Servers :
          \/ Leave(s) \/ StartLoad(s)
          \/ LandAcquire(s) \/ ReplyAcquire(s) \/ FaultAcquire(s)
          \/ LandOwned(s) \/ ReplyOwned(s) \/ FaultOwned(s)
          \/ \E p \in Purchases : Call(s, p) \/ Wait(s, p)
    \/ \E t \in Tokens : SendAcquire(t) \/ Abandon(t) \/ Pump(t) \/ SafeBound(t)

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
(* Properties. README.md says each in plain English.                        *)

\* At most one session is writable at any moment, across every server.
OneAuthority == Cardinality({t \in Tokens : Writable(t)}) <= 1

\* No purchase is granted twice, in the record or in any session's data.
GrantOnce ==
    \A p \in Purchases : store.payload[p] <= 1 /\ \A t \in Tokens : sess[t].value[p] <= 1

\* A grant and its purchase id are stored in one revision: the record holds one exactly when it holds the other.
GrantWithId == \A p \in Purchases : (p \in store.receipts) <=> (store.payload[p] >= 1)

\* Roblox is answered PurchaseGranted only for a purchase whose grant the record holds, and it keeps holding it.
AckedDurable == \A p \in acked : store.payload[p] = 1 /\ p \in store.receipts

\* What a session believes durable, the record holds.
DurableStored == \A t \in Tokens : sess[t].phase \in {"active", "sealed"} => sess[t].durable \subseteq store.receipts

\* afterGrant effects run at most once per purchase, and only for one Roblox was told is granted.
EffectsOnce == \A p \in Purchases : ran[p] <= 1 /\ (ran[p] = 1 => p \in acked)

\* A session's data changes only while it is writable: a sealed or expired session stops writing.
StopsWriting ==
    [][\A t \in Tokens : sess[t].phase \in {"active", "sealed", "ended"} /\ sess'[t].value # sess[t].value
                           => Writable(t)]_vars
=============================================================================
