import { Logging } from "@trioforge/crucible";
import { boot } from "@trioforge/crucible/server";
import { GAME_NAME } from "../shared/game";

boot({ load: [] });
Logging.getLogger("Game.Server").info(`${GAME_NAME} started`);
