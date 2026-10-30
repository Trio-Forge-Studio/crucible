import { Logging } from "@trioforge/crucible";
const log = Logging.getLogger("Sample.Client");
log.debug("S2 game debug {side}", { side: "client" });
log.info("S2 game info {side}", { side: "client" });
log.warn("S2 game warn {side}", { side: "client" });
log.error("S2 game error {side}", { side: "client" });
import { boot, BOOTED_ATTRIBUTE } from "@trioforge/crucible/client";

boot({ load: ["src/client/systems/**"] });
script.SetAttribute("CrucibleClient", game.GetService("Players").LocalPlayer.GetAttribute(BOOTED_ATTRIBUTE));
print("S2 client booted");
