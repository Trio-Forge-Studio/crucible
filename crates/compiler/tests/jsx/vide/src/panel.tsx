import Vide, { For, Show } from "@rbxts/vide";

export function Label(props: { text: string | (() => string); order?: number | (() => number); children?: Vide.Node }) {
	return (
		<textlabel Name="Label" Text={props.text} LayoutOrder={props.order} BackgroundTransparency={1}>
			{props.children}
		</textlabel>
	);
}

export interface PanelProps {
	title: () => string;
	open: () => boolean;
	items: () => string[];
	onClose: () => void;
}

export function Panel(props: PanelProps) {
	const close = { Name: "Close", Text: "x" };
	return (
		<frame Name="Panel" Visible>
			<Label text={props.title} order={1} />
			<textbutton {...close} Activated={props.onClose} />
			<Show when={props.open}>{() => <Label text="open" order={2} />}</Show>
			<For each={props.items}>
				{(item: string, index: () => number) => <Label text={item} order={() => 10 + index()} />}
			</For>
			<>
				<uicorner Name="Corner" />
				<uipadding Name="Padding" />
			</>
		</frame>
	);
}
