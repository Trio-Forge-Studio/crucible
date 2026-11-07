------------------------------ MODULE Reconcile ------------------------------
(***************************************************************************)
(* How one locked resource's fields converge in `crucible provision`.      *)
(* `specs/provisioning` decides which resource the lock names. This model  *)
(* starts from any state that model can leave: any field values, a create  *)
(* marker in the description, and an icon hash that is unknown or true.   *)
(* The run reads the listed fields back from the list endpoint. It cannot  *)
(* read the icon, so the lock keeps the icon's hash. README.md states the  *)
(* properties and how the two models compose.                              *)
(***************************************************************************)
EXTENDS Integers

CONSTANTS
    Fields,
    Vals,
    Icons,
    MaxFaults,
    MaxEdits,
    ListLag,
    LateIcon

NoIcon == 0
ASSUME NoIcon \notin Icons
IdleRun == [pc |-> "idle", obs |-> [f \in Fields |-> CHOOSE v \in Vals : TRUE]]

VARIABLES
    want,
    wantIcon,
    actual,
    actualIcon,
    shown,
    lockIcon,
    late,
    lateIcons,
    run,
    faults,
    edits

roblox == <<actual, actualIcon, shown, late, lateIcons>>
vars == <<want, wantIcon, roblox, lockIcon, run, faults, edits>>

Unbounded == -1
Budget(max, n) == max = Unbounded \/ n < max
Spend(max, n) == IF max = Unbounded THEN n ELSE n + 1
Fault == Budget(MaxFaults, faults) /\ faults' = Spend(MaxFaults, faults)
Stop == run' = IdleRun

Payload == [f |-> want, icon |-> NoIcon]
Land(m) ==
    /\ actual' = m.f
    /\ shown' = IF ListLag THEN shown ELSE m.f
    /\ actualIcon' = IF m.icon = NoIcon THEN actualIcon ELSE m.icon

-----------------------------------------------------------------------------

Start ==
    /\ run.pc = "idle"
    /\ run' = [run EXCEPT !.pc = "listing"]
    /\ UNCHANGED <<want, wantIcon, roblox, lockIcon, faults, edits>>

List ==
    /\ run.pc = "listing"
    /\ \/ run' = [pc |-> "meta", obs |-> shown] /\ UNCHANGED faults
       \/ Fault /\ Stop
    /\ UNCHANGED <<want, wantIcon, roblox, lockIcon, edits>>

SkipMeta ==
    /\ run.pc = "meta"
    /\ run.obs = want
    /\ run' = [run EXCEPT !.pc = "icon"]
    /\ UNCHANGED <<want, wantIcon, roblox, lockIcon, faults, edits>>

UpdateMeta ==
    /\ run.pc = "meta"
    /\ run.obs # want
    /\ \/ /\ Land(Payload)
          /\ run' = [run EXCEPT !.pc = "icon"]
          /\ UNCHANGED <<late, lateIcons, faults>>
       \/ /\ Fault
          /\ Stop
          /\ \/ Land(Payload) /\ UNCHANGED <<late, lateIcons>>
             \/ UNCHANGED roblox
             \/ late' = late \cup {Payload} /\ UNCHANGED <<actual, actualIcon, shown, lateIcons>>
    /\ UNCHANGED <<want, wantIcon, lockIcon, edits>>

WriteIconStale ==
    /\ run.pc = "icon"
    /\ lockIcon \notin {wantIcon, NoIcon}
    /\ lockIcon' = NoIcon
    /\ UNCHANGED <<want, wantIcon, roblox, run, faults, edits>>

UploadIcon ==
    /\ run.pc = "icon"
    /\ lockIcon = NoIcon
    /\ \/ /\ actualIcon' = wantIcon
          /\ run' = [run EXCEPT !.pc = "hash"]
          /\ UNCHANGED <<actual, shown, late, lateIcons, lockIcon, faults>>
       \/ /\ Fault
          /\ Stop
          /\ \/ actualIcon' = wantIcon /\ UNCHANGED <<actual, shown, late, lateIcons>>
             \/ UNCHANGED roblox
             \/ /\ LateIcon
                /\ lateIcons' = lateIcons \cup {wantIcon}
                /\ UNCHANGED <<actual, actualIcon, shown, late>>
          /\ UNCHANGED lockIcon
    /\ UNCHANGED <<want, wantIcon, edits>>

WriteIconHash ==
    /\ run.pc = "hash"
    /\ lockIcon' = wantIcon
    /\ Stop
    /\ UNCHANGED <<want, wantIcon, roblox, faults, edits>>

Finish ==
    /\ run.pc = "icon"
    /\ lockIcon = wantIcon
    /\ Stop
    /\ UNCHANGED <<want, wantIcon, roblox, lockIcon, faults, edits>>

Crash ==
    /\ run.pc # "idle"
    /\ Fault
    /\ Stop
    /\ UNCHANGED <<want, wantIcon, roblox, lockIcon, edits>>

Provision ==
    \/ Start \/ List \/ SkipMeta \/ UpdateMeta
    \/ WriteIconStale \/ UploadIcon \/ WriteIconHash \/ Finish

-----------------------------------------------------------------------------

EditField(f, v) ==
    /\ run.pc = "idle"
    /\ Budget(MaxEdits, edits)
    /\ want[f] # v
    /\ want' = [want EXCEPT ![f] = v]
    /\ edits' = Spend(MaxEdits, edits)
    /\ UNCHANGED <<wantIcon, roblox, lockIcon, run, faults>>

EditIcon(c) ==
    /\ run.pc = "idle"
    /\ Budget(MaxEdits, edits)
    /\ wantIcon # c
    /\ wantIcon' = c
    /\ edits' = Spend(MaxEdits, edits)
    /\ UNCHANGED <<want, roblox, lockIcon, run, faults>>

EditInCreations(f, v) ==
    /\ Budget(MaxEdits, edits)
    /\ actual[f] # v
    /\ Land([f |-> [actual EXCEPT ![f] = v], icon |-> NoIcon])
    /\ edits' = Spend(MaxEdits, edits)
    /\ UNCHANGED <<want, wantIcon, late, lateIcons, lockIcon, run, faults>>

LandLate(m) ==
    /\ m \in late
    /\ late' = late \ {m}
    /\ \/ Land(m)
       \/ UNCHANGED <<actual, actualIcon, shown>>
    /\ UNCHANGED <<want, wantIcon, lateIcons, lockIcon, run, faults, edits>>

LandLateIcon(c) ==
    /\ c \in lateIcons
    /\ lateIcons' = lateIcons \ {c}
    /\ actualIcon' \in {actualIcon, c}
    /\ UNCHANGED <<want, wantIcon, actual, shown, late, lockIcon, run, faults, edits>>

Propagate ==
    /\ ListLag
    /\ shown # actual
    /\ shown' = actual
    /\ UNCHANGED <<want, wantIcon, actual, actualIcon, late, lateIcons, lockIcon, run,
                   faults, edits>>

-----------------------------------------------------------------------------

Init ==
    /\ want = [f \in Fields |-> CHOOSE v \in Vals : TRUE]
    /\ wantIcon = CHOOSE c \in Icons : TRUE
    /\ actual \in [Fields -> Vals]
    /\ actualIcon \in Icons
    /\ shown = actual
    /\ lockIcon \in {NoIcon, actualIcon}
    /\ late = {}
    /\ lateIcons = {}
    /\ run = IdleRun
    /\ faults = 0
    /\ edits = 0

Next ==
    \/ Provision \/ Crash
    \/ \E f \in Fields, v \in Vals : EditField(f, v) \/ EditInCreations(f, v)
    \/ \E c \in Icons : EditIcon(c)
    \/ \E m \in late : LandLate(m)
    \/ \E c \in lateIcons : LandLateIcon(c)
    \/ Propagate

Spec == Init /\ [][Next]_vars

LiveSpec ==
    /\ Spec
    /\ WF_vars(Provision)
    /\ WF_vars(\E m \in late : LandLate(m))
    /\ WF_vars(\E c \in lateIcons : LandLateIcon(c))
    /\ WF_vars(Propagate)

-----------------------------------------------------------------------------

TypeOK ==
    /\ want \in [Fields -> Vals]
    /\ actual \in [Fields -> Vals]
    /\ lockIcon \in Icons \cup {NoIcon}
    /\ run.pc \in {"idle", "listing", "meta", "icon", "hash"}

IconHashTrue == lockIcon # NoIcon => actualIcon = lockIcon

Writes(a, b) == IF a = b THEN 0 ELSE 1

OneWritePerStep == [][Writes(lockIcon, lockIcon') + Writes(roblox, roblox') <= 1]_vars

Converges == <>[](actual = want /\ actualIcon = wantIcon /\ lockIcon = wantIcon)
=============================================================================
