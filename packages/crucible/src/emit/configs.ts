import type { Config, PlayerConfig } from "../configs";
import { declare, readGlobal, readPlayer } from "../internal/configs/runtime";
import type { Node } from "../internal/network/schema";

export function config<T>(name: string, fallback: T, value: Node): Config<T> {
	const declaration = declare(name, "global", fallback, value);
	return (() => readGlobal(declaration)) as unknown as Config<T>;
}

export function playerConfig<T>(name: string, fallback: T, value: Node): PlayerConfig<T> {
	const declaration = declare(name, "player", fallback, value);
	return ((player: Player) => readPlayer(declaration, player)) as unknown as PlayerConfig<T>;
}
