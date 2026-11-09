local process = require("@lune/process")
local loadPlace = require("../../../../test/lib/place")

local world = loadPlace(false, process.args[1])
local main = world.place:GetService("ServerScriptService").TS.main
local value = world.import(main).value
assert(value == 5, `main read {value} from crucible:experience, not 5`)
print("main read 5 from crucible:experience")
