import type { Readable } from "@trioforge/crucible";
import { mount } from "@trioforge/crucible/ui";

export function mountWidth(parent: Instance) {
	return mount(() => <frame Name="Width" className="w-1/2" />, parent);
}

function Tinted(props: { color?: Color3 }) {
	return <frame Name="Tinted" BackgroundColor3={props.color} className="bg-brand" />;
}

export function mountTinted(parent: Instance) {
	return mount(
		() => (
			<>
				<Tinted />
				<Tinted color={Color3.fromRGB(1, 2, 3)} />
			</>
		),
		parent,
	);
}

export function mountStyled(parent: Instance, text: Readable<string>, onBuy: () => void) {
	return mount(
		() => (
			<textbutton
				Name="Styled"
				className="bg-brand/90 text-white text-lg size-16 w-64 rounded-md p-4 px-6 border-white gap-2"
				Text={text}
				Activated={onBuy}
			/>
		),
		parent,
	);
}
