import type { Readable } from "@trioforge/crucible";

export function Before(color: Readable<Color3>, size: Readable<UDim2>) {
	return <frame BackgroundColor3={color} Size={size} className="bg-brand size-4" />;
}
export function After(color: Readable<Color3>, size: Readable<UDim2>) {
	return <frame className="bg-brand size-4" BackgroundColor3={color} Size={size} />;
}
export const fixedBefore = (
	<frame BackgroundColor3={Color3.fromRGB(1, 2, 3)} Size={new UDim2()} className="bg-brand size-4" />
);
export const fixedAfter = (
	<frame className="bg-brand size-4" BackgroundColor3={Color3.fromRGB(1, 2, 3)} Size={new UDim2()} />
);
