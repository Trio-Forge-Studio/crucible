import Vide, { cleanup, effect, read, source, untrack } from "@rbxts/vide";
import { heartbeat } from "./frame-loop";
import type { Children, Value } from "./props";
import { topbarLayout } from "./topbar-policy";
import { useTopbarIcon, type TopbarIconOptions } from "./use-topbar-icon";

const GuiService = game.GetService("GuiService");
const TextService = game.GetService("TextService");

export type TopbarSide = "Left" | "Right";
export interface TopbarRootProps {
	side: TopbarSide;
	displayOrder: number;
	children?: Children;
}
export interface TopbarIconProps extends TopbarIconOptions {
	name: string;
	image: Value<string>;
	imageScale?: Value<number>;
	caption?: Value<string>;
	order: number;
}
export interface TopbarLabelProps {
	name: string;
	text: Value<string>;
	order: number;
}
const rightEdge = 12;
const white = new Color3(1, 1, 1);
const surface = Color3.fromRGB(18, 18, 21);
const labelFont = new Font("rbxasset://fonts/families/BuilderSans.json", Enum.FontWeight.Bold);
const labelTextSize = 16;
const captionColor = Color3.fromRGB(39, 41, 48);
const captionFont = new Font("rbxasset://fonts/families/BuilderSans.json", Enum.FontWeight.Medium);
const chevronSpace = 8;
const captionGap = 12;
const captionLift = 8;
const captionFrequency = 25;
const hiddenOpacity = 0.01;
function measure() {
	const [inset] = GuiService.GetGuiInset();
	return topbarLayout(GuiService.TopbarInset.Height, inset.Y, GuiService.IsTenFootInterface());
}
function watch<T>(property: "TopbarInset" | "PreferredTransparency", read: () => T): () => T {
	const value = source(read());
	cleanup(GuiService.GetPropertyChangedSignal(property).Connect(() => value(read())));
	return value;
}
// X is the caption's opacity and Y its offset in pixels. A critically damped spring fades it in while it slides down,
// and fades it out in place.
function useCaptionMotion(shown: () => boolean): () => Vector2 {
	const motion = source(Vector2.zero);
	let velocity = Vector2.zero;
	let goal = Vector2.zero;
	let stop: (() => void) | undefined;
	cleanup(() => stop?.());
	effect(() => {
		const current = untrack(motion);
		if (shown()) {
			if (current.X <= hiddenOpacity) {
				motion(new Vector2(0, -captionLift));
				velocity = Vector2.zero;
			}
			goal = new Vector2(1, 0);
		} else {
			velocity = Vector2.zero;
			goal = new Vector2(0, current.Y);
		}
		if (stop || untrack(motion) === goal) return;
		stop = heartbeat((_, delta) => {
			const displacement = motion().sub(goal);
			const decay = math.exp(-captionFrequency * delta);
			const drive = velocity.add(displacement.mul(captionFrequency));
			const remaining = displacement.add(drive.mul(delta)).mul(decay);
			velocity = velocity.sub(drive.mul(captionFrequency * delta)).mul(decay);
			if (remaining.Magnitude < 1e-3 && velocity.Magnitude < 1e-3) {
				motion(goal);
				velocity = Vector2.zero;
				stop?.();
				stop = undefined;
			} else motion(goal.add(remaining));
		});
	});
	return motion;
}
function Round() {
	return <uicorner CornerRadius={new UDim(1, 0)} />;
}
function TopbarRoot(props: TopbarRootProps): Vide.Node {
	const layout = watch("TopbarInset", measure);
	const left = props.side === "Left";
	return (
		<screengui
			Name={`Topbar${props.side}`}
			DisplayOrder={props.displayOrder}
			ResetOnSpawn={false}
			ScreenInsets={Enum.ScreenInsets.TopbarSafeInsets}
			ZIndexBehavior={Enum.ZIndexBehavior.Sibling}
		>
			<frame
				Name="Icons"
				BackgroundTransparency={1}
				Position={() => UDim2.fromOffset(0, layout().offsetY)}
				Size={() => new UDim2(1, 0, 1, layout().sizeY)}
			>
				<uilistlayout
					FillDirection={Enum.FillDirection.Horizontal}
					HorizontalAlignment={left ? Enum.HorizontalAlignment.Left : Enum.HorizontalAlignment.Right}
					VerticalAlignment={Enum.VerticalAlignment.Bottom}
					SortOrder={Enum.SortOrder.LayoutOrder}
					Padding={() => new UDim(0, layout().gap)}
				/>
				<uipadding
					PaddingLeft={() => new UDim(0, left ? layout().gap : 0)}
					PaddingRight={new UDim(0, left ? 0 : rightEdge)}
				/>
				{props.children}
			</frame>
		</screengui>
	);
}
function TopbarIcon(props: TopbarIconProps): Vide.Node {
	const icon = useTopbarIcon(props);
	const preferred = watch("PreferredTransparency", () => GuiService.PreferredTransparency);
	const center = new Vector2(0.5, 0.5);
	return (
		<frame Name={props.name} LayoutOrder={props.order} Size={UDim2.fromOffset(44, 44)} BackgroundTransparency={1}>
			<textbutton
				Name="ClickRegion"
				Text=""
				AutoButtonColor={false}
				Size={UDim2.fromScale(1, 1)}
				BackgroundColor3={surface}
				BackgroundTransparency={() => 0.08 * preferred()}
				action={icon.action}
			>
				<Round />
				<frame
					Name="IconSpot"
					AnchorPoint={center}
					Position={UDim2.fromScale(0.5, 0.5)}
					Size={new UDim2(1, -8, 1, -8)}
					BackgroundColor3={white}
					BackgroundTransparency={() => (icon.selected() ? 0.7 : 1)}
				>
					<Round />
					<uigradient
						Enabled={icon.selected}
						Rotation={45}
						Color={new ColorSequence(Color3.fromRGB(96, 98, 100), Color3.fromRGB(77, 78, 80))}
					/>
					<frame
						Name="IconOverlay"
						Size={UDim2.fromScale(1, 1)}
						BackgroundColor3={white}
						BackgroundTransparency={0.85}
						Visible={icon.highlighted}
					>
						<Round />
					</frame>
					<imagelabel
						Name="IconImage"
						AnchorPoint={center}
						Position={UDim2.fromScale(0.5, 0.5)}
						Size={() => {
							const scale = read(props.imageScale ?? 0.5);
							return UDim2.fromScale(scale, scale);
						}}
						BackgroundTransparency={1}
						Image={props.image}
						ZIndex={2}
					/>
				</frame>
			</textbutton>
			<frame
				Name="Notice"
				Visible={() => icon.notice() > 0}
				Position={new UDim2(1, -12, 0, -1)}
				Size={UDim2.fromOffset(20, 20)}
				AutomaticSize={Enum.AutomaticSize.X}
				BackgroundColor3={white}
				BackgroundTransparency={0.1}
				ZIndex={3}
			>
				<Round />
				<uistroke />
				<textlabel
					Size={UDim2.fromScale(1, 1)}
					BackgroundTransparency={1}
					Text={() => (icon.notice() > 99 ? "99+" : tostring(icon.notice()))}
					TextSize={() => (icon.notice() > 99 ? 11 : 13)}
					TextColor3={new Color3(0, 0, 0)}
					Font={Enum.Font.Arial}
				/>
			</frame>
			{props.caption !== undefined ? <TopbarCaption text={props.caption} shown={icon.captionShown} /> : undefined}
		</frame>
	);
}
function TopbarCaption(props: { text: Value<string>; shown: () => boolean }): Vide.Node {
	const motion = useCaptionMotion(props.shown);
	return (
		<canvasgroup
			Name="Caption"
			Visible={() => motion().X > hiddenOpacity}
			GroupTransparency={() => 1 - math.clamp(motion().X, 0, 1)}
			AnchorPoint={new Vector2(0.5, 0)}
			Position={() => new UDim2(0.5, 0, 1, captionGap - chevronSpace + math.round(motion().Y))}
			Size={UDim2.fromOffset(0, 40 + chevronSpace)}
			AutomaticSize={Enum.AutomaticSize.X}
			BackgroundTransparency={1}
			ZIndex={4}
		>
			<frame
				Name="Pill"
				Position={UDim2.fromOffset(0, chevronSpace)}
				Size={UDim2.fromOffset(0, 40)}
				AutomaticSize={Enum.AutomaticSize.X}
				BackgroundColor3={captionColor}
				BorderSizePixel={0}
			>
				<uicorner />
				<frame
					Name="Chevron"
					AnchorPoint={new Vector2(0.5, 0.5)}
					Position={UDim2.fromScale(0.5, 0)}
					Size={UDim2.fromOffset(10, 10)}
					Rotation={45}
					BackgroundColor3={captionColor}
					BorderSizePixel={0}
				/>
				<frame Name="Text" BackgroundTransparency={1} AutomaticSize={Enum.AutomaticSize.XY}>
					<uipadding
						PaddingTop={new UDim(0, 12)}
						PaddingBottom={new UDim(0, 12)}
						PaddingLeft={new UDim(0, 13)}
						PaddingRight={new UDim(0, 13)}
					/>
					<textlabel
						AutomaticSize={Enum.AutomaticSize.X}
						Size={UDim2.fromOffset(0, 16)}
						BackgroundTransparency={1}
						Text={props.text}
						TextSize={15}
						FontFace={captionFont}
						TextColor3={white}
					/>
				</frame>
			</frame>
		</canvasgroup>
	);
}
// Roblox can keep an automatic size when the text changes to a string of the same length, so the label measures it.
function TopbarLabel(props: TopbarLabelProps): Vide.Node {
	const preferred = watch("PreferredTransparency", () => GuiService.PreferredTransparency);
	const width = source(0);
	effect(() => {
		const params = new Instance("GetTextBoundsParams");
		params.Text = read(props.text);
		params.Font = labelFont;
		params.Size = labelTextSize;
		params.Width = math.huge;
		let current = true;
		cleanup(() => {
			current = false;
		});
		task.defer(() => {
			const [ok, bounds] = pcall(() => TextService.GetTextBoundsAsync(params));
			if (ok && current) width(math.ceil(bounds.X));
		});
	});
	return (
		<frame
			Name={props.name}
			LayoutOrder={props.order}
			Size={() => UDim2.fromOffset(math.max(width() + 32, 44), 44)}
			BackgroundColor3={surface}
			BackgroundTransparency={() => 0.08 * preferred()}
		>
			<Round />
			<textlabel
				Name="Label"
				Position={UDim2.fromOffset(15, 0)}
				Size={new UDim2(1, -32, 1, 0)}
				BackgroundTransparency={1}
				Text={props.text}
				TextSize={labelTextSize}
				FontFace={labelFont}
				TextColor3={white}
			/>
		</frame>
	);
}
export const Topbar = { Root: TopbarRoot, Icon: TopbarIcon, Label: TopbarLabel };
