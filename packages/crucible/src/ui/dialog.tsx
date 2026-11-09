import Vide, { show, untrack } from "@rbxts/vide";
import { useFocusScope } from "./focus-scope";
import { buttonHost, frameHost, partContext, type AsChildProps, type ButtonPartProps, type PartContext } from "./part";
import type { ChildFactory, Value } from "./props";
import { closeFallbackContext, ownerContext } from "./runtime";
import { useControllable } from "./use-controllable";
import { usePress, type PressBinding } from "./use-press";

// Dismiss is the close input, such as ButtonB, reaching the open content.
export type DialogOpenChangeReason = "Trigger" | "Close" | "Dismiss";
export type AlertDialogOpenChangeReason = "Action" | "Cancel" | "Dismiss";
export interface DialogBinding {
	open: () => boolean;
}
interface DialogContext<R> extends DialogBinding {
	setOpen: (open: boolean, reason: R) => void;
	destructive: boolean;
}
const dialogContext = partContext<DialogContext<DialogOpenChangeReason>>("Dialog.Root");
const alertContext = partContext<DialogContext<AlertDialogOpenChangeReason>>("AlertDialog.Root");

interface RootProps<R> {
	open?: Value<boolean>;
	defaultOpen?: boolean;
	onOpenChange?: (open: boolean, reason: R) => void;
	children: (dialog: DialogBinding) => Vide.Node;
}
export type DialogRootProps = RootProps<DialogOpenChangeReason>;
function root<R>(context: PartContext<DialogContext<R>>, props: RootProps<R>, destructive: boolean): Vide.Node {
	const [open, write] = useControllable({ value: props.open, defaultValue: props.defaultOpen ?? false });
	const setOpen = (value: boolean, reason: R) => {
		if (value === untrack(open)) return;
		write(value);
		props.onOpenChange?.(value, reason);
	};
	return context.provide({ open, setOpen, destructive }, () => props.children({ open }));
}

export type DialogContentProps = {
	selection?: "Native" | "Pointer";
	restoreFocus?: boolean;
} & AsChildProps<Frame, ChildFactory> & { children: ChildFactory };
function content(dialog: DialogContext<"Dismiss">, props: DialogContentProps, part: string): Vide.Node {
	return show(dialog.open, () => {
		const scope = useFocusScope({
			trapped: true,
			selection: props.selection,
			restoreFocus: props.restoreFocus,
			onClose: () => dialog.setOpen(false, "Dismiss"),
		});
		return ownerContext(scope.owner, () =>
			frameHost(props, part, ["selection", "restoreFocus"], () => props.children(), scope.root, {
				Size: UDim2.fromScale(1, 1),
			}),
		);
	});
}

export type DialogButtonProps = { disabled?: Value<boolean> } & ButtonPartProps<PressBinding>;
export type DialogTriggerProps = DialogButtonProps & { selectFirst?: Value<boolean> };
function DialogTrigger(props: DialogTriggerProps): Vide.Node {
	const dialog = dialogContext.use("Dialog.Trigger");
	const press = usePress({
		onPress: () => dialog.setOpen(true, "Trigger"),
		disabled: props.disabled,
		selectFirst: props.selectFirst,
	});
	return buttonHost(props, "Dialog.Trigger", ["disabled", "selectFirst"], press.action, press);
}

// Close and Cancel are the scope's fallback selection when nothing else is selectable.
function closingButton<R>(
	props: DialogButtonProps & { onPress?: () => void },
	part: string,
	dialog: DialogContext<R>,
	reason: R,
	selectFirst: boolean,
	fallback: boolean,
): Vide.Node {
	let press!: PressBinding;
	closeFallbackContext(fallback, () => {
		press = usePress({
			onPress: () => {
				props.onPress?.();
				dialog.setOpen(false, reason);
			},
			disabled: props.disabled,
			selectFirst,
		});
	});
	return buttonHost(props, part, ["disabled", "onPress"], press.action, press);
}
function DialogClose(props: DialogButtonProps): Vide.Node {
	const dialog = dialogContext.use("Dialog.Close");
	return closingButton(props, "Dialog.Close", dialog, "Close", false, true);
}

export const Dialog = {
	Root: (props: DialogRootProps) => root(dialogContext, props, false),
	Trigger: DialogTrigger,
	Content: (props: DialogContentProps) => content(dialogContext.use("Dialog.Content"), props, "Dialog.Content"),
	Close: DialogClose,
};

export interface AlertDialogRootProps extends RootProps<AlertDialogOpenChangeReason> {
	destructive?: boolean;
}
export type AlertDialogButtonProps = DialogButtonProps & { onPress?: () => void };
function AlertDialogAction(props: AlertDialogButtonProps): Vide.Node {
	const dialog = alertContext.use("AlertDialog.Action");
	return closingButton(props, "AlertDialog.Action", dialog, "Action", !dialog.destructive, false);
}
function AlertDialogCancel(props: AlertDialogButtonProps): Vide.Node {
	const dialog = alertContext.use("AlertDialog.Cancel");
	return closingButton(props, "AlertDialog.Cancel", dialog, "Cancel", dialog.destructive, true);
}

export const AlertDialog = {
	Root: (props: AlertDialogRootProps) => root(alertContext, props, props.destructive ?? false),
	Content: (props: DialogContentProps) =>
		content(alertContext.use("AlertDialog.Content"), props, "AlertDialog.Content"),
	Action: AlertDialogAction,
	Cancel: AlertDialogCancel,
};
