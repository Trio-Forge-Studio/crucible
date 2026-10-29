type AttributeValue = string | number | boolean | Vector3 | Color3;
export function attribute<T extends AttributeValue>(): T;
export function attribute(fallback: number): number;
export function attribute(fallback: string): string;
export function attribute(fallback: boolean): boolean;
export function attribute<T extends AttributeValue | undefined>(fallback: T): T;
export function attribute(_fallback?: AttributeValue): AttributeValue | undefined {
	error("attribute() was not replaced; add the Crucible transformer to the compiler plugins");
}
