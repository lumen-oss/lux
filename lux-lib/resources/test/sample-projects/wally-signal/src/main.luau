local signal = require("@signal")

assert(type(signal) == "table", "signal package did not load")
local instance = signal.new()
assert(signal.Is(instance), "signal.new() returned an invalid signal")
