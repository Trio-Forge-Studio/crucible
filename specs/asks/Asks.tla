-------------------------------- MODULE Asks --------------------------------
(***************************************************************************)
(* One client's asks to the server over Crucible's network session: the    *)
(* client's pending asks, its queue before READY, each ask's timeout, the  *)
(* server's handlers with their lifetime and active count, the retirement  *)
(* of the receiving Subsystem, and either side ending the session.         *)
(* packages/crucible/src/internal/network/client-session.ts,               *)
(* server-session.ts and session.ts implement it.                          *)
(*                                                                         *)
(* The RemoteEvent delivers each side's packets in order and never twice   *)
(* (A1). Packets in flight when a side ends are dropped. A handler may run *)
(* for any time, and a timer fires any time after it was armed.            *)
(* specs/asks/README.md states the assumptions and each property.          *)
(***************************************************************************)
EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS
    Nonces,      \* the asks the client makes, in order
    MaxPending,  \* MAX_PENDING_ASKS
    MaxQueued,   \* MAX_QUEUED_PACKETS before READY
    MaxActive    \* MAX_ACTIVE_HANDLERS

VARIABLES
    cpeer,    \* the client's peer: "waiting" before its commit sends READY, "ready", or "ended"
    pending,  \* pending[n]: "none" before ask n, "waiting" while it waits, "settled" once it has its result
    result,   \* result[n]: what ask n returned, or "none"
    resumed,  \* ghost: resumed[n] counts the results ask n's caller got
    queue,    \* the client's packets held before READY
    c2s,      \* packets on their way to the server
    s2c,      \* packets on their way to the client
    speer,    \* the server's peer for this player
    handler,  \* handler[n]: "none", "running", "overrun" past its lifetime, or "done"
    answered, \* answered[n]: the server sent ask n's reply (the entry is settled)
    counted,  \* the server peer's count of handlers toward MaxActive
    live,     \* the receiving claim is live: its Subsystem has not retired
    last,     \* the highest request nonce the server accepted
    asked,    \* the asks made so far
    replies   \* ghost: replies[n] counts the replies the server sent to ask n

vars == <<cpeer, pending, result, resumed, queue, c2s, s2c, speer, handler, answered, counted, live, last, asked,
          replies>>

Waiting == {n \in Nonces : pending[n] = "waiting"}
Range(s) == {s[i] : i \in 1..Len(s)}
Remove(s, n) == SelectSeq(s, LAMBDA m : m # n)

Request(n) == [type |-> "request", n |-> n]
Reply(n, r) == [type |-> "reply", n |-> n, r |-> r]
ReadyPacket == [type |-> "ready", n |-> 0]
FailPacket == [type |-> "fail", n |-> 0]

\* The caller's result: once, through the waiting entry (settle) or at once from ask.
Settle(n, r) ==
    /\ pending' = [pending EXCEPT ![n] = "settled"]
    /\ result' = [result EXCEPT ![n] = r]
    /\ resumed' = [resumed EXCEPT ![n] = @ + 1]

\* end(peer): every waiting ask gets "disconnected".
EndClient ==
    /\ cpeer' = "ended"
    /\ queue' = <<>>
    /\ pending' = [n \in Nonces |-> IF pending[n] = "waiting" THEN "settled" ELSE pending[n]]
    /\ result' = [n \in Nonces |-> IF pending[n] = "waiting" THEN "disconnected" ELSE result[n]]
    /\ resumed' = [n \in Nonces |-> IF pending[n] = "waiting" THEN resumed[n] + 1 ELSE resumed[n]]

\* Network.ask: refused at once when ended or overloaded, else pending until its reply, its timeout or the end.
Ask ==
    LET n == asked + 1 IN
    /\ n \in Nonces
    /\ asked' = n
    /\ IF cpeer = "ended" THEN
          /\ Settle(n, "disconnected")
          /\ UNCHANGED <<cpeer, queue, c2s>>
       ELSE IF Cardinality(Waiting) >= MaxPending THEN
          /\ Settle(n, "overloaded")
          /\ UNCHANGED <<cpeer, queue, c2s>>
       ELSE IF cpeer = "ready" THEN
          /\ pending' = [pending EXCEPT ![n] = "waiting"]
          /\ c2s' = Append(c2s, Request(n))
          /\ UNCHANGED <<cpeer, result, resumed, queue>>
       ELSE IF Len(queue) < MaxQueued THEN
          /\ pending' = [pending EXCEPT ![n] = "waiting"]
          /\ queue' = Append(queue, n)
          /\ UNCHANGED <<cpeer, result, resumed, c2s>>
       ELSE
          \* An overflow fails the session, which settles every waiting ask, this one included.
          /\ cpeer' = "ended"
          /\ queue' = <<>>
          /\ c2s' = Append(c2s, FailPacket)
          /\ pending' = [m \in Nonces |-> IF pending[m] = "waiting" \/ m = n THEN "settled" ELSE pending[m]]
          /\ result' = [m \in Nonces |-> IF pending[m] = "waiting" \/ m = n THEN "disconnected" ELSE result[m]]
          /\ resumed' = [m \in Nonces |-> IF pending[m] = "waiting" \/ m = n THEN resumed[m] + 1 ELSE resumed[m]]
    /\ UNCHANGED <<s2c, speer, handler, answered, counted, live, last, replies>>

\* The client's commit sends READY and then its queue.
Commit ==
    /\ cpeer = "waiting"
    /\ cpeer' = "ready"
    /\ c2s' = (c2s \o <<ReadyPacket>>) \o [i \in 1..Len(queue) |-> Request(queue[i])]
    /\ queue' = <<>>
    /\ UNCHANGED <<pending, result, resumed, s2c, speer, handler, answered, counted, live, last, asked, replies>>

\* The ask's timer: a queued packet leaves the queue, and the ask times out.
Expire(n) ==
    /\ pending[n] = "waiting"
    /\ queue' = Remove(queue, n)
    /\ Settle(n, "timeout")
    /\ UNCHANGED <<cpeer, c2s, s2c, speer, handler, answered, counted, live, last, asked, replies>>

\* The client closes, or fails the session on a malformed packet.
CloseClient ==
    /\ cpeer # "ended"
    /\ EndClient
    /\ c2s' = Append(c2s, FailPacket)
    /\ UNCHANGED <<s2c, speer, handler, answered, counted, live, last, asked, replies>>

\* A reply settles its ask if it still waits. A late or unknown one changes nothing.
ClientReceive ==
    LET p == Head(s2c) IN
    /\ s2c # <<>>
    /\ s2c' = Tail(s2c)
    /\ IF cpeer = "ended" THEN UNCHANGED <<cpeer, pending, result, resumed, queue>>
       ELSE IF p.type = "fail" THEN EndClient
       ELSE IF pending[p.n] = "waiting" THEN Settle(p.n, p.r) /\ UNCHANGED <<cpeer, queue>>
       ELSE UNCHANGED <<cpeer, pending, result, resumed, queue>>
    /\ UNCHANGED <<c2s, speer, handler, answered, counted, live, last, asked, replies>>

Send(n, r) ==
    /\ s2c' = Append(s2c, Reply(n, r))
    /\ replies' = [replies EXCEPT ![n] = @ + 1]

\* The server reads one packet. A request is refused at once when it cannot run, and starts its handler otherwise.
ServerReceive ==
    LET p == Head(c2s) IN
    /\ c2s # <<>>
    /\ c2s' = Tail(c2s)
    /\ IF speer = "ended" THEN UNCHANGED <<speer, s2c, handler, counted, last, replies>>
       ELSE IF p.type = "ready" THEN
          /\ speer' = IF speer = "waiting" THEN "ready" ELSE speer
          /\ UNCHANGED <<s2c, handler, counted, last, replies>>
       ELSE IF p.type = "fail" THEN
          /\ speer' = "ended"
          /\ UNCHANGED <<s2c, handler, counted, last, replies>>
       ELSE IF speer # "ready" \/ p.n <= last THEN UNCHANGED <<speer, s2c, handler, counted, last, replies>>
       ELSE
          /\ last' = p.n
          /\ \/ /\ Send(p.n, "invalid")
                /\ UNCHANGED <<handler, counted>>
             \/ /\ ~live
                /\ Send(p.n, "unavailable")
                /\ UNCHANGED <<handler, counted>>
             \/ /\ live
                /\ counted >= MaxActive
                /\ Send(p.n, "overloaded")
                /\ UNCHANGED <<handler, counted>>
             \/ /\ live
                /\ counted < MaxActive
                /\ handler' = [handler EXCEPT ![p.n] = "running"]
                /\ counted' = counted + 1
                /\ UNCHANGED <<s2c, replies>>
          /\ UNCHANGED speer
    /\ UNCHANGED <<cpeer, pending, result, resumed, queue, answered, live, asked>>

\* The handler returns or throws. It replies unless the entry is settled or the peer ended.
Finish(n) ==
    /\ handler[n] \in {"running", "overrun"}
    /\ handler' = [handler EXCEPT ![n] = "done"]
    /\ counted' = IF handler[n] = "running" THEN counted - 1 ELSE counted
    /\ IF ~answered[n] /\ speer # "ended"
       THEN /\ answered' = [answered EXCEPT ![n] = TRUE]
            /\ \E r \in {"ok", "handler-error"} : Send(n, r)
       ELSE UNCHANGED <<answered, s2c, replies>>
    /\ UNCHANGED <<cpeer, pending, result, resumed, queue, c2s, speer, live, last, asked>>

\* HANDLER_LIFETIME_SECONDS passed: the handler stops counting and runs on.
Overrun(n) ==
    /\ handler[n] = "running"
    /\ handler' = [handler EXCEPT ![n] = "overrun"]
    /\ counted' = counted - 1
    /\ UNCHANGED <<cpeer, pending, result, resumed, queue, c2s, s2c, speer, answered, live, last, asked, replies>>

\* The receiving Subsystem retires: each request it still runs is answered "unavailable" now.
Retire ==
    LET open == IF speer = "ended" THEN {}
                ELSE {n \in Nonces : handler[n] \in {"running", "overrun"} /\ ~answered[n]}
        order == CHOOSE f \in [1..Cardinality(open) -> open] : \A i, j \in DOMAIN f : i < j => f[i] < f[j]
    IN /\ live
       /\ live' = FALSE
       /\ answered' = [n \in Nonces |-> answered[n] \/ n \in open]
       /\ s2c' = s2c \o [i \in 1..Cardinality(open) |-> Reply(order[i], "unavailable")]
       /\ replies' = [n \in Nonces |-> IF n \in open THEN replies[n] + 1 ELSE replies[n]]
       /\ UNCHANGED <<cpeer, pending, result, resumed, queue, c2s, speer, handler, counted, last, asked>>

\* The player leaves, or the server fails the peer and tells the client.
EndServer ==
    /\ speer # "ended"
    /\ speer' = "ended"
    /\ \/ s2c' = s2c
       \/ s2c' = Append(s2c, FailPacket)
    /\ UNCHANGED <<cpeer, pending, result, resumed, queue, c2s, handler, answered, counted, live, last, asked, replies>>

Init ==
    /\ cpeer = "waiting"
    /\ pending = [n \in Nonces |-> "none"]
    /\ result = [n \in Nonces |-> "none"]
    /\ resumed = [n \in Nonces |-> 0]
    /\ queue = <<>>
    /\ c2s = <<>>
    /\ s2c = <<>>
    /\ speer = "waiting"
    /\ handler = [n \in Nonces |-> "none"]
    /\ answered = [n \in Nonces |-> FALSE]
    /\ counted = 0
    /\ live = TRUE
    /\ last = 0
    /\ asked = 0
    /\ replies = [n \in Nonces |-> 0]

Next ==
    \/ Ask \/ Commit \/ CloseClient \/ ClientReceive
    \/ ServerReceive \/ Retire \/ EndServer
    \/ \E n \in Nonces : Expire(n) \/ Finish(n) \/ Overrun(n)

\* Packets are delivered and every armed timer fires; nothing else is promised.
Fairness ==
    /\ WF_vars(ClientReceive)
    /\ WF_vars(ServerReceive)
    /\ \A n \in Nonces : WF_vars(Expire(n))

Spec == Init /\ [][Next]_vars /\ Fairness

-----------------------------------------------------------------------------
(* Properties. README.md says each in plain English.                        *)

\* Each ask's caller gets one result: its reply, its timeout or the session's end, never two.
OneResult == \A n \in Nonces : resumed[n] <= 1

\* Every ask made is waiting or has its result, and only a waiting ask can still be settled.
Accounted ==
    \A n \in Nonces : /\ (n <= asked) <=> (pending[n] # "none")
                     /\ (pending[n] = "settled") <=> (result[n] # "none")

\* The client never holds more waiting asks than its cap.
Bounded == Cardinality(Waiting) <= MaxPending

\* The server replies to each request at most once.
OneReply == \A n \in Nonces : replies[n] <= 1

\* While the peer lives, its count is exactly its handlers within their lifetime, and never above the cap.
Counted == speer # "ended" => counted = Cardinality({n \in Nonces : handler[n] = "running"}) /\ counted <= MaxActive

\* Every ask that waits gets its result.
Settles == \A n \in Nonces : pending[n] = "waiting" ~> pending[n] = "settled"
=============================================================================
