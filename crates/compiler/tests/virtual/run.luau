local process = require("@lune/process")
local harness = require("../../../../packages/crucible/lune")

local world = harness.loadGame({ place = process.args[1] })
local main = world.place:GetService("ServerScriptService").TS.main
local value = world.import(main).value
assert(value == 5, `main read {value} from crucible:experience, not 5`)
print("main read 5 from crucible:experience")
