import { toClient, toServer } from "@trioforge/crucible";
import type { u8 } from "@trioforge/crucible";

export const Collect = toServer<{ readonly size: u8 }>("Coins.Collect");
export const Collected = toClient<{ readonly gained: number }>("Coins.Collected");
