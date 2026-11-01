import type { Disconnect } from "../port";

export interface ConfigSnapshotPort {
	/** `GetValue`: nil for a missing key. A JSON config may arrive as a table or as its text. */
	readonly get: (key: string) => unknown;
	/** `Refresh`. Yields and may throw. */
	readonly refresh: () => void;
	/** Unmeasured in Studio: whether this turns true without `UpdateAvailable`. Runtime treats the two as independent. */
	readonly outdated: () => boolean;
	/** `Error` is `LoadFailed`. */
	readonly failed: () => boolean;
	readonly onUpdateAvailable: (listener: () => void) => Disconnect;
}

/** What configs ask of the engine. Requests yield and may throw. */
export interface ConfigPort {
	readonly global: () => ConfigSnapshotPort;
	readonly forPlayer: (player: Player) => ConfigSnapshotPort;
	readonly players: () => readonly Player[];
	readonly onPlayerAdded: (listener: (player: Player) => void) => Disconnect;
	readonly onPlayerRemoving: (listener: (player: Player) => void) => Disconnect;
	readonly wait: (seconds: number) => void;
	/** Uniform in [0, 1). */
	readonly random: () => number;
	readonly decodeJson: (text: string) => unknown;
	/** The JSON text length of a decoded value. Throws for a value JSON cannot hold. */
	readonly jsonBytes: (value: object) => number;
}

let port: ConfigPort | undefined;

export function installConfigPort(value: ConfigPort) {
	assert(port === undefined, "Config port already selected");
	port = value;
}

export function configPort(): ConfigPort {
	port ??= createRobloxConfigPort();
	return port;
}

const EMPTY: ConfigSnapshotPort = table.freeze({
	get: () => undefined,
	refresh: () => {},
	outdated: () => false,
	failed: () => false,
	onUpdateAvailable: () => () => {},
});

function snapshotPort(snapshot: ConfigSnapshot): ConfigSnapshotPort {
	return {
		get: (key) => snapshot.GetValue(key),
		refresh: () => snapshot.Refresh(),
		outdated: () => snapshot.Outdated,
		failed: () => snapshot.Error === Enum.ConfigSnapshotErrorState.LoadFailed,
		onUpdateAvailable: (listener) => {
			const connection = snapshot.UpdateAvailable.Connect(listener);
			return () => connection.Disconnect();
		},
	};
}

export function createRobloxConfigPort(): ConfigPort {
	const players = game.GetService("Players");
	const http = game.GetService("HttpService");
	const live =
		!game.GetService("RunService").IsStudio() ||
		game.GetService("ServerStorage").GetAttribute("CrucibleLiveData") === true;
	const service = live ? game.GetService("ConfigService") : undefined;
	const random = new Random();
	return {
		global: () => (service === undefined ? EMPTY : snapshotPort(service.GetConfigAsync())),
		forPlayer: (player) => (service === undefined ? EMPTY : snapshotPort(service.GetConfigForPlayerAsync(player))),
		players: () => players.GetPlayers(),
		onPlayerAdded: (listener) => {
			const connection = players.PlayerAdded.Connect(listener);
			return () => connection.Disconnect();
		},
		onPlayerRemoving: (listener) => {
			const connection = players.PlayerRemoving.Connect(listener);
			return () => connection.Disconnect();
		},
		wait: (seconds) => {
			task.wait(seconds);
		},
		random: () => random.NextNumber(),
		decodeJson: (text) => http.JSONDecode(text),
		jsonBytes: (value) => http.JSONEncode(value).size(),
	};
}
