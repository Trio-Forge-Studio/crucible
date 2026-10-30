import { Logging } from "@trioforge/crucible";
const log = Logging.getLogger("Sample.Server");
log.debug("S2 game debug {side}", { side: "server" });
log.info("S2 game info {side}", { side: "server" });
log.warn("S2 game warn {side}", { side: "server" });
log.error("S2 game error {side}", { side: "server" });
import { boot } from "@trioforge/crucible/server";

boot({ load: ["src/server/systems/**"] });
script.SetAttribute("CrucibleServer", true);
print("S2 server booted");
