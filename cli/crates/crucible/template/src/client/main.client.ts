import { Logging } from "@trioforge/crucible";
import { boot } from "@trioforge/crucible/client";

boot({ load: [] });
Logging.getLogger("Game.Client").info("Game started");
