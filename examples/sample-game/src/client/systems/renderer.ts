import { Subsystem } from "@trioforge/crucible";

export class Renderer extends Subsystem {
	public static readonly renderPriority = 201;
	private frames = 0;
	protected override awake() {
		print("S2 renderer awake");
	}
	protected override start() {
		print("S2 renderer start");
	}
	protected override preRender(_dt: number) {
		this.frames++;
		if (this.frames <= 5) print(`S2 render ${this.frames}`);
	}
}
