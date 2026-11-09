import Vide, { For } from "@rbxts/vide";
import { frameHost, partContext, type AsChildProps, type HostProps } from "./part";
import { toastContext } from "./runtime";
import type { ToastId } from "./toast-state";
import { useToastViewport, type ToastBinding, type ToastSlot, type ToastViewportOptions } from "./use-toast-viewport";

export type ToastViewportProps<D> = ToastViewportOptions<D> &
	Omit<HostProps<Frame>, "children"> & { children: (toast: ToastBinding<D>) => Vide.Node };
export type ToastRootProps = AsChildProps<Frame> & { holdExit?: () => boolean };

const slotContext = partContext<Pick<ToastSlot<unknown>, "root">>("Toast.Viewport");

function ToastViewport<D>(props: ToastViewportProps<D>): Vide.Node {
	const view = useToastViewport(props);
	return frameHost(
		props,
		"Toast.Viewport",
		["toaster", "placement", "layout", "gap", "peek"],
		() => (
			<For each={view.ids}>
				{(id: ToastId) => {
					const slot = view.slot(id);
					return toastContext(slot.inert, () =>
						slotContext.provide(slot, () => props.children(slot.binding)),
					);
				}}
			</For>
		),
		view.viewport,
		{ Size: UDim2.fromScale(1, 1) },
	);
}
function ToastRoot(props: ToastRootProps): Vide.Node {
	const slot = slotContext.use("Toast.Root");
	return frameHost(
		props,
		"Toast.Root",
		["holdExit"],
		() => props.children,
		(host) => slot.root(host, props.holdExit),
		{ AutomaticSize: Enum.AutomaticSize.XY },
	);
}

export const Toast = { Viewport: ToastViewport, Root: ToastRoot };
