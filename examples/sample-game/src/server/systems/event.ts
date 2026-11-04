import { config } from "@trioforge/crucible/server";

export interface EventConfig {
	readonly multiplier: number;
	readonly endsAt: number;
}

export const NO_EVENT: EventConfig = { multiplier: 1, endsAt: 0 };

export const GlobalEvent = config<EventConfig>("GlobalEvent", NO_EVENT);
