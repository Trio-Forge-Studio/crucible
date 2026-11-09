import Vide from "@rbxts/vide";
import { buttonHost, chain, hostProps, partContext, slot, type AsChildProps, type ButtonPartProps } from "./part";
import { useTextField, type TextFieldBinding, type TextFieldOptions } from "./use-text-field";

export type TextFieldRootProps = TextFieldOptions & ButtonPartProps<TextFieldBinding>;
export type TextFieldInputProps = AsChildProps<TextBox>;
const options = [
	"value",
	"defaultValue",
	"onValueChange",
	"onSubmit",
	"onValueCommit",
	"completion",
	"maxLength",
	"autoFocus",
	"disabled",
	"invalid",
	"selectFirst",
];
const context = partContext<TextFieldBinding>("TextField.Root");

function TextFieldRoot(props: TextFieldRootProps): Vide.Node {
	const field = useTextField(props);
	return context.provide(field, () => buttonHost(props, "TextField.Root", options, field.control, field));
}
function TextFieldInput(props: TextFieldInputProps): Vide.Node {
	const field = context.use("TextField.Input");
	if (props.asChild) return slot(props.children, "TextField.Input", "TextBox", field.box);
	return (
		<textbox
			BackgroundTransparency={1}
			BorderSizePixel={0}
			{...hostProps(props, [])}
			action={chain(field.box, props.action)}
		>
			{props.children}
		</textbox>
	);
}

export const TextField = { Root: TextFieldRoot, Input: TextFieldInput };
