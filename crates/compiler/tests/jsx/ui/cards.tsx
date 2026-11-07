import { mount, type Node } from "@trioforge/crucible/ui";

function Card(props: { title: string; children?: Node }) {
	return <frame Name={props.title}>{props.children}</frame>;
}

export function mountCards(parent: Instance) {
	return mount(
		() => (
			<>
				<Card title="A">
					<textlabel Name="Body" Text="a" />
				</Card>
				<Card title="B" />
			</>
		),
		parent,
	);
}
