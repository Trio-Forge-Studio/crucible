import { effect, Logging, receive, state, Subsystem } from "@trioforge/crucible";
import { Network } from "@trioforge/crucible/client";
import { Collect, Collected } from "../../shared/messages";
import { PlayerData } from "../../shared/profile";
import { TotalCoins } from "../../shared/state";

const log = Logging.getLogger("Sample.Hud");
const Players = game.GetService("Players");
const UserInputService = game.GetService("UserInputService");

export class Hud extends Subsystem {
	private readonly lastGain = state(0);

	private readonly collected = receive(Collected, ({ gained }) => {
		this.lastGain.set(gained);
	});

	protected override start() {
		const screen = new Instance("ScreenGui");
		screen.Name = "CoinHud";
		const label = new Instance("TextLabel");
		label.Name = "Coins";
		label.Size = new UDim2(0, 480, 0, 60);
		label.Position = new UDim2(0, 20, 0, 20);
		label.TextSize = 28;
		label.Parent = screen;
		screen.Parent = Players.LocalPlayer.WaitForChild("PlayerGui");
		effect(() => {
			const coins = PlayerData.loaded() ? PlayerData().coins() : 0;
			label.Text = `Coins: ${coins}   Total: ${TotalCoins()}   Last: +${this.lastGain()}`;
			log.info("hud {text}", { text: label.Text });
		});
		UserInputService.InputBegan.Connect((input, processed) => {
			if (!processed && input.KeyCode === Enum.KeyCode.E) Network.send(Collect, { size: 1 });
		});
	}
}
