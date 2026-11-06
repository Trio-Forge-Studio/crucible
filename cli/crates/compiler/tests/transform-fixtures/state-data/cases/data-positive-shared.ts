import { profileStore, replicated, replicatedPerPlayer } from "@trioforge/crucible";
import type { PlayedAnchor, PlayerData as FdPlayerData } from "./fd";

declare const any: never;
type PlayerData = Omit<FdPlayerData, "version" | "purchaseIds">;

export const PlayerProfiles = profileStore<PlayerData>("FD.Profiles.Explicit", { template: any });
export const Played = replicatedPerPlayer<PlayedAnchor | undefined>("Analytics.Played", undefined);
type Phase = { readonly kind: "Idle" } | { readonly kind: "Running"; readonly startedAt: number };
export const Phases = profileStore<{ readonly phase: Phase; readonly note?: { readonly text: string } }>(
	"Game.Phases",
	{ template: any },
);
export const ReplicatedPose = replicated<{ readonly at: Vector3 }>("Probe.Pose", any);
