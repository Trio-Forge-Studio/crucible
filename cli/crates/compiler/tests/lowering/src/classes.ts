export abstract class Base {
	protected readonly tags = new Array<string>();

	constructor(readonly name: string) {
		this.tags.push("base");
	}

	describe(): string {
		return `${this.name}:${this.kind()}`;
	}

	protected abstract kind(): string;

	static make(): string {
		return "static";
	}
}

export class Leaf extends Base {
	count = 1;
	static readonly label = "leaf";

	constructor(
		name: string,
		private readonly extra: number,
	) {
		super(name);
		this.count += extra;
	}

	protected kind() {
		return "leaf";
	}

	override describe() {
		return `${super.describe()}+${this.count}+${this.tags.size()}`;
	}

	end() {
		return this.extra;
	}

	toString() {
		return "Leaf";
	}
}

export function build() {
	const leaf = new Leaf("a", 2);
	return [leaf.describe(), leaf.end(), Leaf.label, Leaf.make(), tostring(leaf)] as const;
}
