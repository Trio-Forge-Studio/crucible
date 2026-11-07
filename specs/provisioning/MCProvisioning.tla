--------------------------- MODULE MCProvisioning ---------------------------
EXTENDS Provisioning, TLC

MCKeys == {"p", "p2", "q", "b"}
MCKind == [k \in MCKeys |-> IF k = "b" THEN "badge" ELSE "product"]
MCName0 == [k \in MCKeys |-> IF k = "q" THEN "gold" ELSE "vip"]
MCMoved == "p2" :> "p"
MCNoMoves == [k \in {} |-> "p"]

MCCount(products, badges) == ("product" :> products) @@ ("badge" :> badges)
MCC00 == MCCount(0, 0)
MCC10 == MCCount(1, 0)
MCC01 == MCCount(0, 1)
MCC02 == MCCount(0, 2)
MCC03 == MCCount(0, 3)
MCC21 == MCCount(2, 1)
MCC31 == MCCount(3, 1)
MCC32 == MCCount(3, 2)

MCUnbounded == Unbounded

MCNonceSymmetry == Permutations(Nonces)
MCSymmetry == Permutations(Provs) \cup Permutations(Nonces)
=============================================================================
