/** @internal A stored value with the platform data UpdateAsync clears unless the transform returns it (rule 41). */
export interface StoredValue {
	readonly value: unknown;
	readonly userIds: readonly number[];
	readonly metadata: { readonly [key: string]: unknown };
}

/** @internal `retryable`: throttle or transient. `ambiguous`: the write may have committed. */
export type StorageOutcome<T> =
	| { readonly kind: "ok"; readonly value: T }
	| { readonly kind: "error"; readonly code: number; readonly retryable: boolean; readonly ambiguous: boolean };

/** @internal Transforms may run several times and never yield; returning undefined cancels. */
export type Transform = (current: StoredValue | undefined) => StoredValue | undefined;

/**
 * @internal
 * Everything data asks of the engine (rule 105). It has no monotonic clock: data reads `bootClock()`, the clock core's
 * shutdown sweep uses (rule 25). `wall` is for leases and id times only.
 */
export interface DataPort {
	readonly update: (store: string, key: string, transform: Transform) => StorageOutcome<StoredValue | undefined>;
	readonly read: (store: string, key: string) => StorageOutcome<StoredValue | undefined>;
	readonly budget: (kind: "update" | "read") => number;
	readonly wall: () => number;
	readonly serverId: string;
	readonly jobId: string;
	readonly guid: () => string;
	readonly random: () => number;
	readonly spawn: (work: () => void) => void;
	readonly delay: (seconds: number, work: () => void) => thread;
	readonly cancel: (thread: thread) => void;
	readonly resume: (thread: thread) => void;
	readonly wait: (seconds: number) => void;
	readonly players: {
		readonly count: () => number;
		readonly present: (player: Player) => boolean;
		readonly kick: (player: Player, message: string) => void;
	};
}

let port: DataPort | undefined;

/** @internal */
export function installDataPort(value: DataPort) {
	assert(port === undefined, "Data port already selected");
	port = value;
}

/** @internal */
export function dataPort(): DataPort {
	port ??= createRobloxDataPort();
	return port;
}

/** @internal 301 to 306 are throttles that sent nothing; 5xx and codeless errors may have committed. */
export function classify(message: unknown): StorageOutcome<never> {
	const [digits] = tostring(message).match("^(%d+):");
	const code = digits === undefined ? 0 : tonumber(digits)!;
	if (code >= 301 && code <= 306) return { kind: "error", code, retryable: true, ambiguous: false };
	if (code === 0 || (code >= 500 && code <= 599)) return { kind: "error", code, retryable: true, ambiguous: true };
	return { kind: "error", code, retryable: false, ambiguous: false };
}

interface Backing {
	readonly update: DataPort["update"];
	readonly read: DataPort["read"];
	readonly budget: DataPort["budget"];
}

function liveBacking(): Backing {
	const service = game.GetService("DataStoreService");
	return {
		update: (store, key, transform) => {
			const [ok, result] = pcall(() => {
				const [value] = service
					.GetDataStore(store)
					.UpdateAsync(key, (value: unknown, info?: DataStoreKeyInfo) => {
						const written = transform(
							value === undefined && info === undefined
								? undefined
								: {
										value,
										userIds: info?.GetUserIds() ?? [],
										metadata: (info?.GetMetadata() ?? {}) as StoredValue["metadata"],
									},
						);
						if (written === undefined) return $tuple(undefined);
						return $tuple(written.value, written.userIds as number[], written.metadata as object);
					});
				return value;
			});
			if (!ok) return classify(result);
			return {
				kind: "ok",
				value: result === undefined ? undefined : { value: result, userIds: [], metadata: {} },
			};
		},
		read: (store, key) => {
			const [ok, value, info] = pcall(() => service.GetDataStore(store).GetAsync(key));
			if (!ok) return classify(value);
			if (value === undefined) return { kind: "ok", value: undefined };
			const keyInfo = info as DataStoreKeyInfo | undefined;
			return {
				kind: "ok",
				value: {
					value,
					userIds: keyInfo?.GetUserIds() ?? [],
					metadata: (keyInfo?.GetMetadata() ?? {}) as StoredValue["metadata"],
				},
			};
		},
		budget: (kind) =>
			service.GetRequestBudgetForRequestType(
				kind === "update" ? Enum.DataStoreRequestType.UpdateAsync : Enum.DataStoreRequestType.GetAsync,
			),
	};
}

/** Studio's store: the same transforms over an in-process table, so Studio runs lease.ts (rule 106). */
function memoryBacking(): Backing {
	const stores = new Map<string, Map<string, StoredValue>>();
	const keys = (store: string) => {
		let found = stores.get(store);
		if (found === undefined) {
			found = new Map();
			stores.set(store, found);
		}
		return found;
	};
	return {
		update: (store, key, transform) => {
			const [ok, result] = pcall(() => {
				const written = transform(keys(store).get(key));
				if (written !== undefined) keys(store).set(key, written);
				return written;
			});
			if (!ok) return { kind: "error", code: 0, retryable: false, ambiguous: false };
			return { kind: "ok", value: result };
		},
		read: (store, key) => ({ kind: "ok", value: keys(store).get(key) }),
		budget: () => math.huge,
	};
}

/** @internal */
export function createRobloxDataPort(): DataPort {
	const runService = game.GetService("RunService");
	const players = game.GetService("Players");
	const http = game.GetService("HttpService");
	const live = !runService.IsStudio() || game.GetService("ServerStorage").GetAttribute("CrucibleLiveData") === true;
	const backing = live ? liveBacking() : memoryBacking();
	const random = new Random();
	return {
		...backing,
		wall: () => DateTime.now().UnixTimestampMillis / 1000,
		serverId: http.GenerateGUID(false),
		jobId: game.JobId,
		guid: () => http.GenerateGUID(false),
		random: () => random.NextNumber(),
		spawn: (work) => {
			task.spawn(work);
		},
		delay: (seconds, work) => task.delay(seconds, work),
		cancel: (thread) => task.cancel(thread),
		resume: (thread) => {
			task.spawn(thread);
		},
		wait: (seconds) => {
			task.wait(seconds);
		},
		players: {
			count: () => players.GetPlayers().size(),
			present: (player) => player.Parent === players,
			kick: (player, message) => player.Kick(message),
		},
	};
}
