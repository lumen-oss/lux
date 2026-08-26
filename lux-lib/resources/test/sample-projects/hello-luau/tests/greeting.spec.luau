return function(tiniest)
	local test = tiniest.test
	local expect = tiniest.expect

	test("says hello", function()
		local greeting = require("../src/greeting")
		expect(greeting.hello("luau")).is("hello from luau")
	end)

	test("greets others", function()
		local greeting = require("../src/greeting")
		expect(greeting.hello("world")).is("hello from world")
	end)
end
