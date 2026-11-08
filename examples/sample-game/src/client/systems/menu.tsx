import { atom } from "@rbxts/charm";
import Vide, { mount, source } from "@rbxts/vide";
import { effect, own, Subsystem, watch } from "@trioforge/crucible";
import { Checkbox, Dialog, installAnvil, modalOpen, startMenuRuntime, type InputDevice } from "@trioforge/crucible/ui";

const Players = game.GetService("Players");
const UserInputService = game.GetService("UserInputService");

const device = source<InputDevice>("Keyboard");

function onButtonB(signal: RBXScriptSignal<(input: InputObject) => void>, listener: () => void) {
	const connection = signal.Connect((input) => {
		if (input.KeyCode === Enum.KeyCode.ButtonB) listener();
	});
	return () => connection.Disconnect();
}

function Settings(props: { music: boolean; onMusicChange: (on: boolean) => void }): Vide.Node {
	return (
		<Dialog.Root>
			{() => [
				<frame Name="Settings" Position={UDim2.fromOffset(20, 100)} Size={UDim2.fromOffset(180, 96)}>
					<uilistlayout Padding={new UDim(0, 8)} />
					<Checkbox.Root
						Name="Music"
						defaultChecked={props.music}
						onCheckedChange={props.onMusicChange}
						Size={UDim2.fromOffset(40, 40)}
					>
						{() => <Checkbox.Indicator Size={UDim2.fromScale(1, 1)} BackgroundTransparency={0} />}
					</Checkbox.Root>
					<Dialog.Trigger Name="Help" Text="Help" Size={UDim2.fromOffset(120, 40)} />
				</frame>,
				<Dialog.Content>
					{() => (
						<frame
							Name="HelpPanel"
							AnchorPoint={new Vector2(0.5, 0.5)}
							Position={UDim2.fromScale(0.5, 0.5)}
							Size={UDim2.fromOffset(320, 160)}
						>
							<textlabel Text="Press E to collect a coin." Size={new UDim2(1, 0, 0, 40)} />
							<Dialog.Close
								Name="Done"
								Text="Done"
								Position={UDim2.fromOffset(100, 100)}
								Size={UDim2.fromOffset(120, 40)}
							/>
						</frame>
					)}
				</Dialog.Content>,
			]}
		</Dialog.Root>
	);
}

export class Menu extends Subsystem {
	private readonly shown = atom(false);
	private readonly music = atom(true);

	protected override start() {
		installAnvil({
			device: () => device(),
			close: {
				held: () => UserInputService.IsGamepadButtonDown(Enum.UserInputType.Gamepad1, Enum.KeyCode.ButtonB),
				onPressed: (listener) => onButtonB(UserInputService.InputBegan, listener),
				onReleased: (listener) => onButtonB(UserInputService.InputEnded, listener),
			},
			claimMenuInput: () => () => {},
			feedback: () => {},
		});
		own(startMenuRuntime());
		own(
			UserInputService.LastInputTypeChanged.Connect((input) => {
				if (input === Enum.UserInputType.Gamepad1) device("Gamepad");
				else if (input === Enum.UserInputType.Touch) device("Touch");
				else device("Keyboard");
			}),
		);
		own(
			UserInputService.InputBegan.Connect((input, processed) => {
				if (!processed && input.KeyCode === Enum.KeyCode.M) this.shown((shown) => !shown);
			}),
		);
		effect(() => print(`Sample menu modal ${modalOpen()}`));
		watch(this.shown, (shown) => {
			if (!shown) return;
			const screen = own(new Instance("ScreenGui"));
			screen.Name = "SettingsMenu";
			screen.Parent = Players.LocalPlayer.WaitForChild("PlayerGui");
			own(mount(() => <Settings music={this.music()} onMusicChange={(on) => this.music(on)} />, screen));
		});
	}
}
