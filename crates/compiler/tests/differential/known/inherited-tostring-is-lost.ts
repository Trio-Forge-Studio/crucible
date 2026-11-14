declare function trace(value: unknown, shape: string): void;
class Base {
	toString(): string {
		return "base";
	}
}
class Derived extends Base {}
trace(`${new Derived()}`, "s");
