import { replicated } from "@trioforge/crucible";

export const TotalCoins = replicated<number>("Coins.Total", 0);
