import Vide, { effect } from "@rbxts/vide";
import { buttonHost, frameHost, partContext, type AsChildProps, type ButtonPartProps } from "./part";
import { useSlider, type SliderBinding, type SliderOptions } from "./use-slider";

export type SliderRootProps = SliderOptions & ButtonPartProps<SliderBinding>;
export type SliderPartProps = AsChildProps<Frame>;
const options = [
	"value",
	"defaultValue",
	"min",
	"max",
	"step",
	"onValueChange",
	"onValueCommit",
	"disabled",
	"selectFirst",
	"silent",
];
const context = partContext<SliderBinding>("Slider.Root");

function SliderRoot(props: SliderRootProps): Vide.Node {
	const slider = useSlider(props);
	return context.provide(slider, () => buttonHost(props, "Slider.Root", options, slider.control, slider));
}
function SliderTrack(props: SliderPartProps): Vide.Node {
	const slider = context.use("Slider.Track");
	return frameHost(props, "Slider.Track", [], () => props.children, slider.track);
}
function SliderRange(props: SliderPartProps): Vide.Node {
	const slider = context.use("Slider.Range");
	return frameHost(
		props,
		"Slider.Range",
		[],
		() => props.children,
		(instance) => effect(() => (instance.Size = new UDim2(slider.fraction(), 0, 1, 0))),
	);
}
function SliderThumb(props: SliderPartProps): Vide.Node {
	const slider = context.use("Slider.Thumb");
	return frameHost(
		props,
		"Slider.Thumb",
		[],
		() => props.children,
		(instance) => effect(() => (instance.Position = UDim2.fromScale(slider.fraction(), 0.5))),
		{ AnchorPoint: new Vector2(0.5, 0.5) },
	);
}

export const Slider = { Root: SliderRoot, Track: SliderTrack, Range: SliderRange, Thumb: SliderThumb };
