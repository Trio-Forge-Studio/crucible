import Vide, { cleanup } from "@rbxts/vide";
import type { ChildFactory } from "./props";
import {
	active,
	attachScope,
	closeScope,
	holdRestore,
	modalOpen,
	openScope,
	ownerContext,
	textEditing,
	type Owner,
} from "./runtime";
import { chain, hostProps, type HostProps } from "./part";
export { modalOpen };

export type FocusScopeOptions =
	| { trapped: true; selection?: "Native" | "Pointer"; onClose?: () => void; restoreFocus?: boolean }
	| { trapped?: false; restoreFocus?: boolean };
export interface FocusScopeBinding {
	owner: Owner;
	root: (instance: GuiObject) => void;
}
export function useFocusScope(options: FocusScopeOptions): FocusScopeBinding {
	if (options.trapped) {
		const id = openScope(options.selection ?? "Native", options.onClose, options.restoreFocus ?? true);
		cleanup(() => closeScope(id));
		return {
			owner: id,
			root: (instance) => {
				instance.SelectionGroup = true;
				instance.SelectionBehaviorUp = Enum.SelectionBehavior.Stop;
				instance.SelectionBehaviorDown = Enum.SelectionBehavior.Stop;
				instance.SelectionBehaviorLeft = Enum.SelectionBehavior.Stop;
				instance.SelectionBehaviorRight = Enum.SelectionBehavior.Stop;
				cleanup(attachScope(id, instance));
			},
		};
	}
	const owner = ownerContext();
	const release = options.restoreFocus === false ? undefined : holdRestore();
	let host: GuiObject | undefined;
	cleanup(() => release?.(host));
	return {
		owner,
		root: (instance) => {
			host = instance;
		},
	};
}

export type FocusScopeProps = FocusScopeOptions & Omit<HostProps<Frame>, "children"> & { children: ChildFactory };
export function FocusScope(props: FocusScopeProps): Vide.Node {
	const scope = useFocusScope(props);
	const frame = hostProps(props as FocusScopeProps & { selection?: unknown; onClose?: unknown }, [
		"trapped",
		"restoreFocus",
		"selection",
		"onClose",
	]);
	return ownerContext(scope.owner, () => (
		<frame
			Name="FocusScope"
			Size={UDim2.fromScale(1, 1)}
			BackgroundTransparency={1}
			{...frame}
			action={chain(scope.root, props.action)}
		>
			{props.children()}
		</frame>
	));
}
export function useModalActive(): () => boolean {
	const owner = ownerContext();
	return () => active(owner);
}
export function useTextEditing(): () => boolean {
	return () => textEditing();
}
