# Crucible

Crucible is a TypeScript game framework for Roblox, built by Trio Forge Studio.

You write a game as Subsystems, which are classes that `boot` constructs on the server and the client. Crucible connects them with typed messages and replicated state, saves player data, and builds UI from JSX. It checks the framework's rules when the game compiles, so a `boot` glob that matches no module fails the build instead of leaving a Subsystem unbooted in Studio.

## See it in code

Declare a message and a replicated value once, in shared code.

```ts
// src/shared/coins.ts
import { replicated, toServer } from "@trioforge/crucible";
import type { u8 } from "@trioforge/crucible";

export const Collect = toServer<{ readonly size: u8 }>("Coins.Collect");
export const TotalCoins = replicated<number>("Coins.Total", 0);
```

The server receives the message and writes the value.

```ts
// src/server/systems/wallet.ts
import { receive, Subsystem } from "@trioforge/crucible";
import { publish } from "@trioforge/crucible/server";
import { Collect, TotalCoins } from "../../shared/coins";

export class Wallet extends Subsystem {
	private readonly total = publish(TotalCoins);

	private readonly collect = receive(Collect, (_player, { size }) => {
		this.total.update((total) => total + size);
	});
}
```

Every client reads the value and reruns the effect when it changes.

```ts
// src/client/systems/hud.ts
import { effect, Subsystem } from "@trioforge/crucible";
import { Network } from "@trioforge/crucible/client";
import { Collect, TotalCoins } from "../../shared/coins";

export class Hud extends Subsystem {
	protected override start() {
		effect(() => print(`Total coins: ${TotalCoins()}`));
		Network.send(Collect, { size: 1 });
	}
}
```

The server entry script starts its Subsystems with `boot`.

```ts
// src/server/main.server.ts
import { boot } from "@trioforge/crucible/server";

boot({ load: ["src/server/systems/**"] });
```

The client entry does the same with `@trioforge/crucible/client` and `src/client/systems/**`.

## Start a game

You need Node.js and Roblox Studio. The CLI bundles Rojo.

```sh
npx @trioforge/crucible-cli create my-game
cd my-game
npx crucible build
```

Open `build/start.rbxl` in Studio. [The sample game](examples/sample-game) uses a message, a replicated value, a profile store, and a config together, and [its guide](docs/sample-game.md) walks through it.

Crucible is at version 0.3, so a minor release can change the API.

## Read the guides

The `crucible` CLI carries every guide. `npx crucible docs` lists them, and `npx crucible docs <topic>` opens one. The same files are in [`docs`](docs):

- Structure a game with [boot](docs/boot.md), [lifecycle](docs/lifecycle.md), [reactive state](docs/reactive.md), and [logging](docs/logging.md).
- Talk between the server and clients with [networking](docs/networking.md) and [replicated state](docs/replicated.md).
- Save and load data with [profile stores](docs/profile-stores.md), [data stores](docs/data-stores.md), and [configs](docs/configs.md).
- Build UI with [Vide](docs/vide.md).
- Draw seeded numbers and curves with [random](docs/random.md) and [spline](docs/spline.md).
- Build, test, and ship with the [CLI](docs/cli.md), [Open Cloud](docs/oc.md), [testing](docs/testing.md), and the [compile errors](docs/diagnostics.md).

## Contribute

Read [CONTRIBUTING.md](CONTRIBUTING.md) to set up the workspace and open a pull request. Everyone in the project follows the [code of conduct](CODE_OF_CONDUCT.md).

## License

Crucible is under the [MIT license](LICENSE).
