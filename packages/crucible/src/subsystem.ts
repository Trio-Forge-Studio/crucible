import { CrucibleObject } from "./crucible-object";
import { installSubsystemHooks } from "./internal/hooks";
export abstract class Subsystem extends CrucibleObject {
	static {
		installSubsystemHooks({ onShutdown: (o) => o.onShutdown() });
	}
	protected onShutdown(): void {}
}
