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
