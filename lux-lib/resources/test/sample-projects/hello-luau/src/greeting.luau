local greeting = {}

function greeting.hello(name: string): string
	return `hello from {name}`
end

return greeting
