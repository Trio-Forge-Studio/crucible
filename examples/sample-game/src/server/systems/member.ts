import { attribute, CrucibleBehaviour } from "@trioforge/crucible";

export class Member extends CrucibleBehaviour<"S2Member", Part> {
	public readonly identity = attribute<string>();
	public readonly speed = attribute(17);
	protected override awake() {
		print(`S2 member awake ${this.identity} ${this.speed}`);
	}
	protected override start() {
		print("S2 member start");
	}
	protected override onEnable() {
		print("S2 member enable");
	}
	protected override onDisable() {
		print("S2 member disable");
	}
	protected override onDestroy() {
		print("S2 member destroy");
	}
}
