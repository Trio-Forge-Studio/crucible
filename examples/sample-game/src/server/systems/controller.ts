import { Subsystem } from "@trioforge/crucible";
import { Store } from "./store";

export class Controller extends Subsystem {
	public constructor(private readonly store: Store) {
		super();
	}
	protected override awake() {
		assert(this.store.ready, "S2 dependency must be awake");
		print("S2 controller awake");
	}
	protected override start() {
		print("S2 controller start");
	}
	protected override onShutdown() {
		print("S2 controller shutdown");
	}
}
