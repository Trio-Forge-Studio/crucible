import { Subsystem, CrucibleBehaviour, attribute, inject, type All } from "@trioforge/crucible";
import { boot } from "@trioforge/crucible/server";

export interface Parent {
	parent(): void;
}
export interface Child extends Parent {
	child(): void;
}
export abstract class Base extends Subsystem implements Child {
	parent() {}
	child() {}
}
export class Store extends Base {}
export class Jobs extends Subsystem {
	constructor(
		readonly store: Store,
		readonly all: All<Child>,
	) {
		super();
	}
}
export abstract class Openable<Tag extends string> extends CrucibleBehaviour<Tag, Model> {
	readonly fleetId = attribute<string>();
	constructor(readonly store: Store) {
		super();
	}
}
export class Door extends Openable<"Door"> {
	readonly maxSpeed = attribute(30);
	readonly label = attribute<string | undefined>(undefined);
	readonly mode = attribute<"walk" | "drive">("walk");
	readonly negative = attribute<-2>(-2);
	readonly tint = attribute(new Color3());
	readonly offset = attribute(new Vector3());
	readonly open = attribute(false);
}
const TAG = "ConstTag";
type AliasTag = "AliasTag";
export class ConstTag extends CrucibleBehaviour<typeof TAG> {}
export class AliasedTag extends CrucibleBehaviour<AliasTag, BasePart | Model> {}
const Alias = Openable<"Alias">;
export class ThroughAlias extends Alias {}
class Lookalike {
	value = 1;
}
export class Unrelated extends Lookalike {}
export const one = inject<Store>();
export const many = inject<All<Child>>();
type Many = All<Child>;
export const aliasedMany: All<Child> = inject<Many>();
type FixedCollection<_Unused> = All<Child>;
export const genericAlias: All<Child> = inject<FixedCollection<Store>>();
boot({ load: ["src/server/**/*{Service,Behaviour}.ts"] });
