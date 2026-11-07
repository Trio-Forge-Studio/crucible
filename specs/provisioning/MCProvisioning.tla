--------------------------- MODULE MCProvisioning ---------------------------
(* Constants for the TLC configurations. *)
EXTENDS Provisioning, TLC

\* `coin-pack` is renamed to `coin-pack-2` with `moved_from`. The badge shares
\* the product's display name, as a "VIP" pass and a "VIP" badge would.
MCKind == [k \in {"p", "p2", "b"} |-> IF k = "b" THEN "badge" ELSE "product"]
MCBase == [k \in {"p", "p2", "b"} |-> IF k = "p2" THEN "vip2" ELSE "vip"]
MCMoved == "p2" :> "p"
MCNoMoves == [k \in {} |-> "p"]

MCUnbounded == -1

MCSymmetry == Permutations(Provs)
=============================================================================
