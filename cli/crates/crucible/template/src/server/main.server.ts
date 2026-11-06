import { Logging } from "@trioforge/crucible";
import { boot } from "@trioforge/crucible/server";

boot({ load: [] });
Logging.getLogger("Game.Server").info("Game started");
