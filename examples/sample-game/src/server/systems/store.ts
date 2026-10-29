import { Subsystem } from "@trioforge/crucible";

export class Store extends Subsystem {
	public ready = false;
	protected override awake() {
		this.ready = true;
		print("S2 store awake");
	}
	protected override onShutdown() {
		print("S2 store shutdown");
	}
}
