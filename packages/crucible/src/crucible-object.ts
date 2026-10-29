import { readClaim } from "./internal/construction-claim";
import { FRAME_HOOK_NAMES, FrameHook, installObjectHooks } from "./internal/hooks";

export abstract class CrucibleObject {
	static {
		installObjectHooks({
			awake: (o) => o.awake(),
			start: (o) => o.start(),
			frame: {
				[FrameHook.PreRender]: (o, dt) => o.preRender(dt),
				[FrameHook.FixedUpdate]: (o, dt) => o.fixedUpdate(dt),
				[FrameHook.Update]: (o, dt) => o.update(dt),
				[FrameHook.LateUpdate]: (o, dt) => o.lateUpdate(dt),
			},
			overrides: (klass) => {
				const methods = klass as unknown as Record<string, unknown>;
				const defaults = CrucibleObject as unknown as Record<string, unknown>;
				let mask = 0;
				FRAME_HOOK_NAMES.forEach((name, hook) => {
					if (methods[name] !== defaults[name]) mask |= 1 << hook;
				});
				return mask;
			},
		});
	}

	static readonly renderPriority?: number;

	constructor() {
		const state = readClaim() ?? error("CrucibleObject constructed outside Crucible");
		state.object = this;
	}

	protected awake(): void {}

	protected start(): void {}

	protected fixedUpdate(_dt: number): void {}

	protected update(_dt: number): void {}

	protected lateUpdate(_dt: number): void {}

	protected preRender(_dt: number): void {}
}
