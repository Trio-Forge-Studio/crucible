import type { Readable } from "@trioforge/crucible";
import { mount } from "@trioforge/crucible/ui";

function Button(props: { label?: string; size?: Readable<number>; onClick?: () => void }) {
	return <textbutton Name="Button" Text={props.label} TextSize={props.size} Activated={props.onClick} />;
}

export function mountButtons(parent: Instance, size: Readable<number>, click: () => void) {
	return mount(
		() => (
			<>
				<Button />
				<Button label="Buy" size={size} onClick={click} />
			</>
		),
		parent,
	);
}
