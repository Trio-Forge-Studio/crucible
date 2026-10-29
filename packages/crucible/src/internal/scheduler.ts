import { DenseList } from "./dense-list";
import { FrameHook } from "./hooks";
import { within } from "./lifecycle";
import { Hook, type LifeObject, type LifeRecord, type ObjectState } from "./state";
import { FrameEvent, LogLevel, type Disconnect, type EnginePort } from "./port";

/** @internal */
export interface SchedulerRecord extends LifeRecord {
	readonly id: string;
	readonly hooks: number;
	readonly renderPriority?: number;
}

/** @internal */
export type FrameCallers = { readonly [H in FrameHook]: (object: LifeObject, dt: number) => void };

/** @internal */
export interface SchedulerConfig {
	readonly fixedRate: number;

	readonly renders: boolean;
}

/** @internal */
export const MAX_FIXED_STEPS = 4;

/** @internal */
export const MAX_FRAME_SECONDS = 0.25;

/** @internal */
export function accumulate(carry: number, frameDt: number, step: number): LuaTuple<[steps: number, carry: number]> {
	const total = carry + math.clamp(frameDt, 0, MAX_FRAME_SECONDS);
	const whole = math.floor(total / step);
	return $tuple(math.min(whole, MAX_FIXED_STEPS), math.max(total - whole * step, 0));
}

class Lane {
	readonly members = new DenseList<ObjectState>();
	dt = 0;
	disconnect?: Disconnect;
	readonly visit: (s: ObjectState) => void;

	constructor(hook: FrameHook, frame: FrameCallers) {
		const caller = (s: ObjectState, dt: number) => frame[hook](s.object!, dt);
		this.visit = (s) => within(s, s, Hook.Frame, caller, s, this.dt);
	}

	run(dt: number) {
		this.dt = dt;
		this.members.forEach(this.visit);
	}
}

/** @internal */
export class Scheduler {
	private readonly lanes: { readonly [H in FrameHook]: Lane };
	private readonly renderLanes = new Map<number, Lane>();
	private readonly refusedClasses = new Set<string>();
	private readonly step: number;
	private carry = 0;
	private postSimulation?: Disconnect;

	constructor(
		private readonly port: EnginePort,
		private readonly config: SchedulerConfig,
		private readonly frame: FrameCallers,
	) {
		this.step = 1 / config.fixedRate;
		this.lanes = {
			[FrameHook.PreRender]: new Lane(FrameHook.PreRender, frame),
			[FrameHook.FixedUpdate]: new Lane(FrameHook.FixedUpdate, frame),
			[FrameHook.Update]: new Lane(FrameHook.Update, frame),
			[FrameHook.LateUpdate]: new Lane(FrameHook.LateUpdate, frame),
		};
	}

	add(s: ObjectState) {
		const record = s.record as SchedulerRecord;
		const mask = record.hooks;
		if (
			!this.config.renders &&
			((mask & (1 << FrameHook.PreRender)) !== 0 || record.renderPriority !== undefined)
		) {
			if (!this.refusedClasses.has(record.id)) {
				this.refusedClasses.add(record.id);
				this.port.log(LogLevel.Error, `Crucible server refuses preRender and renderPriority for ${record.id}`);
			}
			return;
		}
		if ((mask & (1 << FrameHook.PreRender)) !== 0) this.join(this.renderLaneOf(s), FrameHook.PreRender, s);
		for (const hook of [FrameHook.FixedUpdate, FrameHook.Update, FrameHook.LateUpdate]) {
			if ((mask & (1 << hook)) !== 0) this.join(this.lanes[hook], hook, s);
		}
	}

	remove(s: ObjectState) {
		const priority = (s.record as SchedulerRecord).renderPriority;
		const render = priority === undefined ? this.lanes[FrameHook.PreRender] : this.renderLanes.get(priority);
		if (render !== undefined && render.members.remove(s) && render.members.size() === 0) {
			render.disconnect?.();
			render.disconnect = undefined;
		}
		const fixed = this.lanes[FrameHook.FixedUpdate];
		if (fixed.members.remove(s) && fixed.members.size() === 0) {
			fixed.disconnect?.();
			fixed.disconnect = undefined;
			this.carry = 0;
		}
		const update = this.lanes[FrameHook.Update];
		const late = this.lanes[FrameHook.LateUpdate];
		const changed = update.members.remove(s);
		if ((late.members.remove(s) || changed) && update.members.size() === 0 && late.members.size() === 0) {
			this.postSimulation?.();
			this.postSimulation = undefined;
		}
	}

	private renderLaneOf(s: ObjectState): Lane {
		const priority = (s.record as SchedulerRecord).renderPriority;
		if (priority === undefined) return this.lanes[FrameHook.PreRender];
		let lane = this.renderLanes.get(priority);
		if (lane === undefined) {
			lane = new Lane(FrameHook.PreRender, this.frame);
			this.renderLanes.set(priority, lane);
		}
		return lane;
	}

	private join(lane: Lane, hook: FrameHook, s: ObjectState) {
		if (!lane.members.add(s)) return;
		const port = this.port;
		switch (hook) {
			case FrameHook.PreRender: {
				if (lane.disconnect !== undefined || !this.config.renders) return;
				const priority = (s.record as SchedulerRecord).renderPriority;
				const run = (dt: number) => lane.run(dt);
				lane.disconnect =
					priority === undefined
						? port.onFrame(FrameEvent.PreRender, run)
						: port.bindRenderStep(`crucible:${priority}`, priority, run);
				return;
			}
			case FrameHook.FixedUpdate:
				if (lane.disconnect !== undefined) return;
				lane.disconnect = port.onFrame(FrameEvent.PreSimulation, (dt) => {
					const [steps, carry] = accumulate(this.carry, dt, this.step);
					this.carry = carry;
					for (let i = 0; i < steps; i++) lane.run(this.step);
				});
				return;
			case FrameHook.Update:
			case FrameHook.LateUpdate: {
				if (this.postSimulation !== undefined) return;
				const update = this.lanes[FrameHook.Update];
				const late = this.lanes[FrameHook.LateUpdate];
				this.postSimulation = port.onFrame(FrameEvent.PostSimulation, (dt) => {
					update.run(dt);
					late.run(dt);
				});
				return;
			}
		}
	}
}
