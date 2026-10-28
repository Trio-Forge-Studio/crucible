local loadSpec: (string) -> { [string]: () -> () } = require

require("@tooling/test")(function(name)
	return loadSpec(`../test/{name}`)
end)
