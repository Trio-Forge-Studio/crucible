declare const anyValue: any;
declare const union: "flex" | "border";
const wide: string = "flex";
const object = { card: "flex" as const };
const getter = {
	get card(): "flex" {
		return "flex";
	},
};
export const views = [
	<frame className={anyValue} />,
	<frame className={union} />,
	<frame className={wide} />,
	<frame className={object.card} />,
	<frame className={getter.card} />,
];
