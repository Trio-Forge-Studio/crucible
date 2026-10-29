import { CrucibleObject } from "./crucible-object";
import { boundTo, defaultAccepts, refresh, remove } from "./internal/binding";
import { readClaim } from "./internal/construction-claim";
import { installBehaviourHooks } from "./internal/hooks";
import { isLive, request, stateOf } from "./internal/lifecycle";
import { Phase, Want } from "./internal/state";

export type BehaviourClass<T extends CrucibleBehaviour<string>> = new (...args: never[]) => T;

export abstract class CrucibleBehaviour<Tag extends string, I extends Instance = Instance> extends CrucibleObject {
	static {
		installBehaviourHooks({
			onEnable: (b) => (b as CrucibleBehaviour<string>).onEnable(),
			onDisable: (b) => (b as CrucibleBehaviour<string>).onDisable(),
			onDestroy: (b) => (b as CrucibleBehaviour<string>).onDestroy(),
		});
	}
	static accepts(instance: Instance): boolean {
		return defaultAccepts(instance);
	}
	static refresh(instance: Instance): void {
		refresh(instance);
	}
	static get<T extends CrucibleBehaviour<string>>(this: BehaviourClass<T>, instance: Instance): T | undefined {
		const state = boundTo(this, instance);
		return state !== undefined && isLive(state) ? (state.object as T) : undefined;
	}
	readonly instance: I;
	declare readonly tag: Tag;
	constructor() {
		super();
		const state = readClaim() ?? error("CrucibleBehaviour constructed outside Crucible");
		this.instance = state.instance as I;
	}
	isEnabled(): boolean {
		const state = stateOf(this);
		return state !== undefined && state.phase === Phase.Enabled && isLive(state);
	}
	setEnabled(enabled: boolean): void {
		const state = stateOf(this);
		if (state !== undefined) request(state, enabled ? Want.Enabled : Want.Disabled);
	}
	remove(): void {
		const state = stateOf(this);
		if (state !== undefined) remove(state);
	}
	protected onEnable(): void {}
	protected onDisable(): void {}
	protected onDestroy(): void {}
}
