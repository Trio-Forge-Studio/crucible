import { External } from "./base";
export class Same {
	static run() {}
	run() {}
}
export class StaticField {
	static run = 1;
	run() {}
}
export class InstanceField {
	run = 1;
	static run() {}
}
export class Parent {
	static get() {}
	value = 1;
}
export class Middle extends Parent {}
export class Child extends Middle {
	get() {}
	static value() {}
}
export class Imported extends External {
	get() {}
	static value() {}
}
export const Expression = class extends Middle {
	get() {}
};
export class Literal {
	static run() {}
	run() {}
}
export class Private {
	private static run() {}
	run() {}
}
export class InstanceAccessor {
	get value() {
		return 1;
	}
	static value() {}
}
export class StaticAccessor {
	static get value() {
		return 1;
	}
	value() {}
}
export class InstanceMethodBase {
	work() {}
}
export class StaticOverride extends InstanceMethodBase {
	static work() {}
}
export class StaticFieldBase {
	readonly neighbor = 2;
	static field = 1;
}
export class InstanceOverride extends StaticFieldBase {
	field() {}
}
