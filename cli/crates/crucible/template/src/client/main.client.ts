import { Logging } from "@trioforge/crucible";
import { boot } from "@trioforge/crucible/client";
import { GAME_NAME } from "../shared/game";

boot({ load: [] });
Logging.getLogger("Game.Client").info(`${GAME_NAME} started`);
