use std::collections::BTreeSet;
use std::str::FromStr;

use super::analyze::Analysis;
use super::LuauTranspileError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Shim {
    TableCreate,
    TableFind,
    TableFreeze,
    TableIsFrozen,
    TableClone,
    TableClear,
    TableMove,
    TablePack,
    TableUnpack,
    StringSplit,
    StringPack,
    StringPackSize,
    StringUnpack,
    StringPackHelpers,
    MathRound,
    MathSign,
    MathClamp,
    CoroutineIsYieldable,
    Typeof,
    Gcinfo,
    Bit32,
    Utf8,
}

/// Prepends the Luau standard library shims required by the given Lua source.
pub(super) fn inject(lua: &str) -> Result<String, LuauTranspileError> {
    let analysis =
        Analysis::new(lua).map_err(|err| LuauTranspileError::Transpile(err.to_string()))?;
    if !analysis.unsupported.is_empty() {
        return Err(LuauTranspileError::Unsupported(
            analysis.unsupported.join(", "),
        ));
    }
    let prelude = analysis
        .shims
        .into_iter()
        .flat_map(|shim| std::iter::once(shim).chain(shim.deps().iter().copied()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|shim| shim.source())
        .collect::<String>();
    Ok(format!("{prelude}{lua}"))
}

impl Shim {
    /// The Lua source of this shim.
    pub(super) fn source(self) -> &'static str {
        match self {
            Shim::TableCreate => TABLE_CREATE,
            Shim::TableFind => TABLE_FIND,
            Shim::TableFreeze => TABLE_FREEZE,
            Shim::TableIsFrozen => TABLE_IS_FROZEN,
            Shim::TableClone => TABLE_CLONE,
            Shim::TableClear => TABLE_CLEAR,
            Shim::TableMove => TABLE_MOVE,
            Shim::TablePack => TABLE_PACK,
            Shim::TableUnpack => TABLE_UNPACK,
            Shim::StringSplit => STRING_SPLIT,
            Shim::StringPack => STRING_PACK,
            Shim::StringPackSize => STRING_PACK_SIZE,
            Shim::StringUnpack => STRING_UNPACK,
            Shim::StringPackHelpers => STRING_PACK_HELPERS,
            Shim::MathRound => MATH_ROUND,
            Shim::MathSign => MATH_SIGN,
            Shim::MathClamp => MATH_CLAMP,
            Shim::CoroutineIsYieldable => COROUTINE_IS_YIELDABLE,
            Shim::Typeof => TYPEOF,
            Shim::Gcinfo => GCINFO,
            Shim::Bit32 => BIT32,
            Shim::Utf8 => UTF8,
        }
    }

    /// Shims that this shim depends on.
    pub(super) fn deps(self) -> &'static [Shim] {
        match self {
            Shim::StringPack | Shim::StringPackSize | Shim::StringUnpack => {
                &[Shim::StringPackHelpers]
            }
            _ => &[],
        }
    }
}

impl FromStr for Shim {
    type Err = ();

    fn from_str(symbol: &str) -> Result<Self, Self::Err> {
        if symbol.starts_with("bit32.") {
            return Ok(Shim::Bit32);
        }
        if symbol.starts_with("utf8.") {
            return Ok(Shim::Utf8);
        }
        match symbol {
            "table.create" => Ok(Shim::TableCreate),
            "table.find" => Ok(Shim::TableFind),
            "table.freeze" => Ok(Shim::TableFreeze),
            "table.isfrozen" => Ok(Shim::TableIsFrozen),
            "table.clone" => Ok(Shim::TableClone),
            "table.clear" => Ok(Shim::TableClear),
            "table.move" => Ok(Shim::TableMove),
            "table.pack" => Ok(Shim::TablePack),
            "table.unpack" => Ok(Shim::TableUnpack),
            "string.split" => Ok(Shim::StringSplit),
            "string.pack" => Ok(Shim::StringPack),
            "string.packsize" => Ok(Shim::StringPackSize),
            "string.unpack" => Ok(Shim::StringUnpack),
            "math.round" => Ok(Shim::MathRound),
            "math.sign" => Ok(Shim::MathSign),
            "math.clamp" => Ok(Shim::MathClamp),
            "coroutine.isyieldable" => Ok(Shim::CoroutineIsYieldable),
            "typeof" => Ok(Shim::Typeof),
            "gcinfo" => Ok(Shim::Gcinfo),
            "bit32" => Ok(Shim::Bit32),
            "utf8" => Ok(Shim::Utf8),
            _ => Err(()),
        }
    }
}

const TABLE_CREATE: &str = r##"if not table.create then
table.create = function(size, value)
	local t = {}
	if size > 0 then
		return setmetatable(t, { __index = function() return value end })
	end
	return t
end
end
"##;

const TABLE_FIND: &str = r##"if not table.find then
table.find = function(t, value, init)
	for i = init or 1, #t do
		if t[i] == value then
			return i
		end
	end
end
end
"##;

const TABLE_FREEZE: &str = r##"if not table.freeze then
table.freeze = function(t)
	local mt = getmetatable(t)
	if mt == nil then
		mt = {}
		setmetatable(t, mt)
	end
	mt.__newindex = function()
		error("attempt to modify a frozen table", 3)
	end
	return t
end
end
"##;

const TABLE_IS_FROZEN: &str = r##"if not table.isfrozen then
table.isfrozen = function(t)
	local mt = getmetatable(t)
	return mt ~= nil and mt.__newindex ~= nil
end
end
"##;

const TABLE_CLONE: &str = r##"if not table.clone then
table.clone = function(t)
	local copy = {}
	for k, v in pairs(t) do
		copy[k] = v
	end
	return setmetatable(copy, getmetatable(t))
end
end
"##;

const TABLE_CLEAR: &str = r##"if not table.clear then
table.clear = function(t)
	for k in pairs(t) do
		t[k] = nil
	end
end
end
"##;

const TABLE_MOVE: &str = r##"if not table.move then
table.move = function(a1, f, e, t, a2)
	a2 = a2 or a1
	if e >= f then
		for i = e, f, -1 do
			a2[t + i - f] = a1[i]
		end
	end
	return a2
end
end
"##;

const TABLE_PACK: &str = r##"if not table.pack then
table.pack = function(...)
	return { n = select("#", ...), ... }
end
end
"##;

const TABLE_UNPACK: &str = r##"if not table.unpack then
table.unpack = function(list, i, j)
	return unpack(list, i, j)
end
end
"##;

const STRING_SPLIT: &str = r##"if not string.split then
string.split = function(s, sep)
	local result = {}
	if sep == nil or sep == "" then
		for i = 1, #s do
			result[#result + 1] = s:sub(i, i)
		end
		return result
	end
	local start = 1
	while true do
		local pos = string.find(s, sep, start, true)
		if pos then
			result[#result + 1] = s:sub(start, pos - 1)
			start = pos + #sep
		else
			result[#result + 1] = s:sub(start)
			break
		end
	end
	return result
end
end
"##;

const MATH_ROUND: &str = r##"if not math.round then
math.round = function(x)
	if x >= 0 then
		return math.floor(x + 0.5)
	end
	return math.ceil(x - 0.5)
end
end
"##;

const MATH_SIGN: &str = r##"if not math.sign then
math.sign = function(x)
	if x > 0 then
		return 1
	end
	if x < 0 then
		return -1
	end
	return 0
end
end
"##;

const MATH_CLAMP: &str = r##"if not math.clamp then
math.clamp = function(x, min, max)
	if x < min then
		return min
	end
	if x > max then
		return max
	end
	return x
end
end
"##;

const COROUTINE_IS_YIELDABLE: &str = r##"if not coroutine.isyieldable then
coroutine.isyieldable = function(co)
	if co == nil then
		return coroutine.running() ~= nil
	end
	return true
end
end
"##;

const TYPEOF: &str = r##"if not typeof then
typeof = function(v)
	local t = type(v)
	if t == "table" or t == "userdata" then
		local mt = getmetatable(v)
		if mt and mt.__type then
			return mt.__type
		end
	end
	return t
end
end
"##;

const GCINFO: &str = r##"if not gcinfo then
gcinfo = function()
	return math.floor(collectgarbage("count"))
end
end
"##;

const BIT32: &str = r##"if not bit32 then
bit32 = {}
local floor = math.floor
local MOD = 4294967296

local function tou(n)
	return floor(n) % MOD
end

local function band2(a, b)
	a = tou(a)
	b = tou(b)
	local p = 1
	local r = 0
	while a > 0 and b > 0 do
		if a % 2 == 1 and b % 2 == 1 then
			r = r + p
		end
		a = floor(a / 2)
		b = floor(b / 2)
		p = p * 2
	end
	return r
end

local function bor2(a, b)
	a = tou(a)
	b = tou(b)
	local p = 1
	local r = 0
	while a > 0 or b > 0 do
		if a % 2 == 1 or b % 2 == 1 then
			r = r + p
		end
		a = floor(a / 2)
		b = floor(b / 2)
		p = p * 2
	end
	return r
end

local function bxor2(a, b)
	a = tou(a)
	b = tou(b)
	local p = 1
	local r = 0
	while a > 0 or b > 0 do
		if (a % 2) ~= (b % 2) then
			r = r + p
		end
		a = floor(a / 2)
		b = floor(b / 2)
		p = p * 2
	end
	return r
end

local function lshift(a, n)
	a = tou(a)
	n = n % 32
	for _ = 1, n do
		a = (a * 2) % MOD
	end
	return a
end

local function rshift(a, n)
	return floor(tou(a) / 2 ^ (n % 32))
end

local function arshift(a, n)
	n = n % 32
	local u = tou(a)
	if n == 0 then
		return u
	end
	if u >= 2147483648 then
		return tou(floor((u - MOD) / 2 ^ n))
	end
	return floor(u / 2 ^ n)
end

bit32.band = function(a, b, ...)
	local r = band2(a, b)
	for i = 1, select("#", ...) do
		r = band2(r, (select(i, ...)))
	end
	return r
end

bit32.bor = function(a, b, ...)
	local r = bor2(a, b)
	for i = 1, select("#", ...) do
		r = bor2(r, (select(i, ...)))
	end
	return r
end

bit32.bxor = function(a, b, ...)
	local r = bxor2(a, b)
	for i = 1, select("#", ...) do
		r = bxor2(r, (select(i, ...)))
	end
	return r
end

bit32.bnot = function(a)
	return MOD - 1 - tou(a)
end

bit32.lshift = lshift
bit32.rshift = rshift
bit32.arshift = arshift

bit32.lrotate = function(a, disp)
	disp = disp % 32
	if disp == 0 then
		return tou(a)
	end
	return bor2(lshift(a, disp), rshift(a, 32 - disp))
end

bit32.rrotate = function(a, disp)
	disp = disp % 32
	if disp == 0 then
		return tou(a)
	end
	return bor2(rshift(a, disp), lshift(a, 32 - disp))
end

bit32.extract = function(n, field, width)
	width = width or 1
	return band2(rshift(n, field), lshift(1, width) - 1)
end

bit32.replace = function(n, v, field, width)
	width = width or 1
	local mask = lshift(1, width) - 1
	return bor2(band2(n, bit32.bnot(lshift(mask, field))), lshift(band2(v, mask), field))
end

bit32.btest = function(a, b, ...)
	local r = band2(a, b)
	for i = 1, select("#", ...) do
		r = band2(r, (select(i, ...)))
	end
	return r ~= 0
end
end
"##;

const UTF8: &str = r##"if not utf8 then
utf8 = {}
local floor = math.floor

local function sequence_length(s, i)
	local b = s:byte(i)
	if b < 128 then
		return 1
	elseif b < 224 then
		return 2
	elseif b < 240 then
		return 3
	elseif b < 248 then
		return 4
	end
	error("invalid UTF-8 byte sequence", 2)
end

local function decode(s, i)
	local b = s:byte(i)
	if b < 128 then
		return b
	elseif b < 224 then
		return (b - 192) * 64 + s:byte(i + 1) - 128
	elseif b < 240 then
		return (b - 224) * 4096 + (s:byte(i + 1) - 128) * 64 + s:byte(i + 2) - 128
	end
	return (b - 240) * 262144 + (s:byte(i + 1) - 128) * 4096 + (s:byte(i + 2) - 128) * 64 + s:byte(i + 3) - 128
end

utf8.charpattern = "[%z\1-\127\194-\244][\128-\191]*"

utf8.char = function(...)
	local out = {}
	for i = 1, select("#", ...) do
		local cp = floor((select(i, ...)))
		if cp < 128 then
			out[#out + 1] = string.char(cp)
		elseif cp < 2048 then
			out[#out + 1] = string.char(192 + floor(cp / 64), 128 + cp % 64)
		elseif cp < 65536 then
			out[#out + 1] = string.char(224 + floor(cp / 4096), 128 + floor(cp / 64) % 64, 128 + cp % 64)
		else
			out[#out + 1] = string.char(240 + floor(cp / 262144), 128 + floor(cp / 4096) % 64, 128 + floor(cp / 64) % 64, 128 + cp % 64)
		end
	end
	return table.concat(out)
end

utf8.codepoint = function(s, i, j)
	i = i or 1
	j = j or i
	local out = {}
	local pos = i
	while pos <= j do
		out[#out + 1] = decode(s, pos)
		pos = pos + sequence_length(s, pos)
	end
	return (table.unpack or unpack)(out)
end

utf8.len = function(s, i, j)
	i = i or 1
	j = j or #s
	local n = 0
	local pos = i
	while pos <= j do
		n = n + 1
		pos = pos + sequence_length(s, pos)
	end
	return n
end

utf8.offset = function(s, n, i)
	i = i or 1
	if n == 0 then
		local pos = i
		while pos > 1 and s:byte(pos) >= 128 and s:byte(pos) < 192 do
			pos = pos - 1
		end
		return pos
	end
	if n > 0 then
		local pos = i
		for _ = 2, n do
			pos = pos + sequence_length(s, pos)
		end
		return pos
	end
	local pos = i
	for _ = 1, -n do
		pos = pos - 1
		while pos > 1 and s:byte(pos) >= 128 and s:byte(pos) < 192 do
			pos = pos - 1
		end
	end
	return pos
end

utf8.codes = function(s)
	local function iter(s, pos)
		if pos >= #s then
			return nil
		end
		pos = pos + 1
		local cp = decode(s, pos)
		return pos + sequence_length(s, pos) - 1, cp
	end
	return iter, s, 0
end
end
"##;

const STRING_PACK: &str = r##"if not string.pack then
string.pack = function(fmt, ...)
	local ops = string.pack_ops(fmt)
	local out = {}
	local arg = 1
	for _, op in ipairs(ops) do
		if op.code == "x" then
			out[#out + 1] = "\0"
		elseif op.code == "c" then
			local s = (select(arg, ...)) or ""
			out[#out + 1] = s:sub(1, op.size)
			arg = arg + 1
		elseif op.code == "z" then
			local s = (select(arg, ...)) or ""
			out[#out + 1] = s
			out[#out + 1] = "\0"
			arg = arg + 1
		elseif op.code == "s" then
			local s = (select(arg, ...)) or ""
			out[#out + 1] = string.pack_int(#s, op.prefix_size, op.big)
			out[#out + 1] = s
			arg = arg + 1
		else
			local n = (select(arg, ...))
			out[#out + 1] = string.pack_int(n, op.size, op.big, op.signed)
			arg = arg + 1
		end
	end
	return table.concat(out)
end
end
"##;

const STRING_PACK_SIZE: &str = r##"if not string.packsize then
string.packsize = function(fmt)
	local total = 0
	for _, op in ipairs(string.pack_ops(fmt)) do
		if op.code == "z" or op.code == "x" then
			total = total + 1
		elseif op.code == "s" then
			total = total + op.prefix_size
		else
			total = total + op.size
		end
	end
	return total
end
end
"##;

const STRING_UNPACK: &str = r##"if not string.unpack then
string.unpack = function(fmt, s, pos)
	pos = pos or 1
	local results = {}
	local nres = 0
	for _, op in ipairs(string.pack_ops(fmt)) do
		if op.code == "x" then
			pos = pos + 1
		elseif op.code == "c" then
			nres = nres + 1
			results[nres] = s:sub(pos, pos + op.size - 1)
			pos = pos + op.size
		elseif op.code == "z" then
			local e = pos
			while s:byte(e) ~= 0 do
				e = e + 1
			end
			nres = nres + 1
			results[nres] = s:sub(pos, e - 1)
			pos = e + 1
		elseif op.code == "s" then
			local len = string.unpack_int(s, pos, op.prefix_size, op.big, false)
			pos = pos + op.prefix_size
			nres = nres + 1
			results[nres] = s:sub(pos, pos + len - 1)
			pos = pos + len
		else
			nres = nres + 1
			results[nres] = string.unpack_int(s, pos, op.size, op.big, op.signed)
			pos = pos + op.size
		end
	end
	return (table.unpack or unpack)(results, 1, nres), pos
end
end
"##;

const STRING_PACK_HELPERS: &str = r##"if not string.pack_ops then
local floor = math.floor

local fixed_sizes = {
	b = { size = 1, signed = true },
	B = { size = 1 },
	h = { size = 2, signed = true },
	H = { size = 2 },
	l = { size = 4, signed = true },
	L = { size = 4 },
	j = { size = 8, signed = true },
	J = { size = 8 },
	T = { size = 8 },
}

string.pack_ops = function(fmt)
	local ops = {}
	local big = false
	local i = 1
	while i <= #fmt do
		local c = fmt:sub(i, i)
		if c == "<" then
			big = false
			i = i + 1
		elseif c == ">" then
			big = true
			i = i + 1
		elseif c == "=" or c == "!" or c == " " then
			i = i + 1
		elseif c == "x" then
			ops[#ops + 1] = { code = "x" }
			i = i + 1
		elseif c == "z" then
			ops[#ops + 1] = { code = "z" }
			i = i + 1
		elseif c == "c" or c == "s" then
			local j = i + 1
			local n = ""
			while fmt:sub(j, j):match("%d") do
				n = n .. fmt:sub(j, j)
				j = j + 1
			end
			if c == "s" then
				ops[#ops + 1] = { code = "s", prefix_size = tonumber(n) or 4, big = big }
			else
				ops[#ops + 1] = { code = "c", size = tonumber(n) or 1 }
			end
			i = j
		elseif c == "i" or c == "I" then
			local j = i + 1
			local n = ""
			while fmt:sub(j, j):match("%d") do
				n = n .. fmt:sub(j, j)
				j = j + 1
			end
			ops[#ops + 1] = { code = "i", size = tonumber(n), big = big, signed = c == "i" }
			i = j
		elseif fixed_sizes[c] then
			local f = fixed_sizes[c]
			ops[#ops + 1] = { code = "i", size = f.size, big = big, signed = f.signed }
			i = i + 1
		else
			error("unsupported string.pack format option '" .. c .. "'", 2)
		end
	end
	return ops
end

string.pack_int = function(n, size, big, signed)
	n = floor(n)
	local bytes = {}
	for _ = 1, size do
		local byte = n % 256
		bytes[#bytes + 1] = byte
		n = (n - byte) / 256
	end
	if big then
		local reversed = {}
		for k = #bytes, 1, -1 do
			reversed[#reversed + 1] = bytes[k]
		end
		bytes = reversed
	end
	return string.char((table.unpack or unpack)(bytes))
end

string.unpack_int = function(s, pos, size, big, signed)
	local n = 0
	if big then
		for i = pos, pos + size - 1 do
			n = n * 256 + s:byte(i)
		end
	else
		for i = pos + size - 1, pos, -1 do
			n = n * 256 + s:byte(i)
		end
	end
	if signed and n >= 2 ^ (size * 8 - 1) then
		n = n - 2 ^ (size * 8)
	end
	return n
end
end
"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn lua_with(shims: &[Shim]) -> mlua::Lua {
        let lua = mlua::Lua::new();
        let src = shims.iter().map(|shim| shim.source()).collect::<String>();
        lua.load(&src).exec().unwrap();
        lua
    }

    #[test]
    fn table_create_returns_default() {
        let lua = lua_with(&[Shim::TableCreate]);
        let value: f64 = lua
            .load("local t = table.create(5, 0) return t[10]")
            .eval()
            .unwrap();
        assert_eq!(value, 0.0);
        let len: f64 = lua.load("return #table.create(5, 0)").eval().unwrap();
        assert_eq!(len, 0.0);
    }

    #[test]
    fn table_find_finds_value() {
        let lua = lua_with(&[Shim::TableFind]);
        let hit: f64 = lua
            .load("return table.find({10, 20, 30}, 20)")
            .eval()
            .unwrap();
        assert_eq!(hit, 2.0);
        let miss: mlua::Value = lua
            .load("return table.find({10, 20, 30}, 99)")
            .eval()
            .unwrap();
        assert!(miss.is_nil());
    }

    #[test]
    fn table_freeze_prevents_writes() {
        let lua = lua_with(&[Shim::TableFreeze, Shim::TableIsFrozen]);
        let (is_frozen, ok): (bool, bool) = lua
            .load("local t = {} table.freeze(t) local ok = pcall(function() t.x = 1 end) return table.isfrozen(t), ok")
            .eval()
            .unwrap();
        assert!(is_frozen);
        assert!(!ok);
        let not_frozen: bool = lua.load("return table.isfrozen({})").eval().unwrap();
        assert!(!not_frozen);
    }

    #[test]
    fn table_clone_copies_and_keeps_metatable() {
        let lua = lua_with(&[Shim::TableClone]);
        let (elem, inherited, same_mt): (f64, f64, bool) = lua
            .load("local t = setmetatable({1, 2, 3}, {__index = {a = 9}}) local c = table.clone(t) return c[3], c.a, getmetatable(c) == getmetatable(t)")
            .eval()
            .unwrap();
        assert_eq!(elem, 3.0);
        assert_eq!(inherited, 9.0);
        assert!(same_mt);
    }

    #[test]
    fn table_clear_empties_table() {
        let lua = lua_with(&[Shim::TableClear]);
        let empty: bool = lua
            .load("local t = {1, 2, 3} table.clear(t) return next(t) == nil")
            .eval()
            .unwrap();
        assert!(empty);
    }

    #[test]
    fn table_move_moves_range() {
        let lua = lua_with(&[Shim::TableMove]);
        let out: String = lua
            .load("local t = {1, 2, 3, 4, 5} table.move(t, 1, 2, 4, t) return table.concat(t, ',')")
            .eval()
            .unwrap();
        assert_eq!(out, "1,2,3,1,2");
    }

    #[test]
    fn table_pack_and_unpack() {
        let lua = lua_with(&[Shim::TablePack, Shim::TableUnpack]);
        let (n, first): (f64, f64) = lua
            .load("local t = table.pack(1, 2, 3) return t.n, t[1]")
            .eval()
            .unwrap();
        assert_eq!(n, 3.0);
        assert_eq!(first, 1.0);
        let sum: f64 = lua
            .load("local a, b = table.unpack({10, 20}) return a + b")
            .eval()
            .unwrap();
        assert_eq!(sum, 30.0);
    }

    #[test]
    fn string_split_literal_and_empty_separator() {
        let lua = lua_with(&[Shim::StringSplit]);
        let comma: String = lua
            .load("return table.concat(string.split('a,b,c', ','), '|')")
            .eval()
            .unwrap();
        assert_eq!(comma, "a|b|c");
        let dot: String = lua
            .load("return table.concat(string.split('a.b', '.'), '|')")
            .eval()
            .unwrap();
        assert_eq!(dot, "a|b");
        let chars: String = lua
            .load("return table.concat(string.split('abc', ''), '|')")
            .eval()
            .unwrap();
        assert_eq!(chars, "a|b|c");
        let empty_field: String = lua
            .load("return table.concat(string.split('1,,2', ','), '|')")
            .eval()
            .unwrap();
        assert_eq!(empty_field, "1||2");
    }

    #[test]
    fn math_round_sign_clamp() {
        let lua = lua_with(&[Shim::MathRound, Shim::MathSign, Shim::MathClamp]);
        assert_eq!(
            lua.load("return math.round(1.5)").eval::<f64>().unwrap(),
            2.0
        );
        assert_eq!(
            lua.load("return math.round(-1.5)").eval::<f64>().unwrap(),
            -2.0
        );
        assert_eq!(
            lua.load("return math.sign(-3)").eval::<f64>().unwrap(),
            -1.0
        );
        assert_eq!(lua.load("return math.sign(0)").eval::<f64>().unwrap(), 0.0);
        assert_eq!(lua.load("return math.sign(7)").eval::<f64>().unwrap(), 1.0);
        assert_eq!(
            lua.load("return math.clamp(5, 1, 3)")
                .eval::<f64>()
                .unwrap(),
            3.0
        );
        assert_eq!(
            lua.load("return math.clamp(0, 1, 3)")
                .eval::<f64>()
                .unwrap(),
            1.0
        );
        assert_eq!(
            lua.load("return math.clamp(2, 1, 3)")
                .eval::<f64>()
                .unwrap(),
            2.0
        );
    }

    #[test]
    fn coroutine_is_yieldable() {
        let lua = lua_with(&[Shim::CoroutineIsYieldable]);
        let (main, in_coroutine): (bool, bool) = lua
            .load("local co = coroutine.create(function() return coroutine.isyieldable() end) local ok, v = coroutine.resume(co) return coroutine.isyieldable(), v")
            .eval()
            .unwrap();
        assert!(!main);
        assert!(in_coroutine);
    }

    #[test]
    fn typeof_classifies() {
        let lua = lua_with(&[Shim::Typeof]);
        assert_eq!(
            lua.load("return typeof(1)").eval::<String>().unwrap(),
            "number"
        );
        assert_eq!(
            lua.load("return typeof('x')").eval::<String>().unwrap(),
            "string"
        );
        assert_eq!(
            lua.load("return typeof(nil)").eval::<String>().unwrap(),
            "nil"
        );
        let custom: String = lua
            .load("return typeof(setmetatable({}, {__type = 'Vector3'}))")
            .eval()
            .unwrap();
        assert_eq!(custom, "Vector3");
    }

    #[test]
    fn gcinfo_returns_a_number() {
        let lua = lua_with(&[Shim::Gcinfo]);
        let value: f64 = lua.load("return gcinfo()").eval().unwrap();
        assert!(value >= 0.0);
    }

    #[test]
    fn bit32_binary_ops() {
        let lua = lua_with(&[Shim::Bit32]);
        assert_eq!(
            lua.load("return bit32.band(255, 15)")
                .eval::<f64>()
                .unwrap(),
            15.0
        );
        assert_eq!(
            lua.load("return bit32.bor(240, 15)").eval::<f64>().unwrap(),
            255.0
        );
        assert_eq!(
            lua.load("return bit32.bxor(255, 15)")
                .eval::<f64>()
                .unwrap(),
            240.0
        );
        assert_eq!(
            lua.load("return bit32.bnot(0)").eval::<f64>().unwrap(),
            4294967295.0
        );
        assert_eq!(
            lua.load("return bit32.bnot(4294967295)")
                .eval::<f64>()
                .unwrap(),
            0.0
        );
    }

    #[test]
    fn bit32_shift_and_rotate() {
        let lua = lua_with(&[Shim::Bit32]);
        assert_eq!(
            lua.load("return bit32.lshift(1, 4)").eval::<f64>().unwrap(),
            16.0
        );
        assert_eq!(
            lua.load("return bit32.rshift(256, 4)")
                .eval::<f64>()
                .unwrap(),
            16.0
        );
        assert_eq!(
            lua.load("return bit32.arshift(2147483648, 4)")
                .eval::<f64>()
                .unwrap(),
            4160749568.0
        );
        assert_eq!(
            lua.load("return bit32.arshift(4294967295, 1)")
                .eval::<f64>()
                .unwrap(),
            4294967295.0
        );
        assert_eq!(
            lua.load("return bit32.lrotate(1, 1)")
                .eval::<f64>()
                .unwrap(),
            2.0
        );
        assert_eq!(
            lua.load("return bit32.rrotate(1, 1)")
                .eval::<f64>()
                .unwrap(),
            2147483648.0
        );
    }

    #[test]
    fn bit32_extract_replace_test() {
        let lua = lua_with(&[Shim::Bit32]);
        assert_eq!(
            lua.load("return bit32.extract(43981, 4, 4)")
                .eval::<f64>()
                .unwrap(),
            12.0
        );
        assert_eq!(
            lua.load("return bit32.replace(0, 43981, 8, 8)")
                .eval::<f64>()
                .unwrap(),
            52480.0
        );
        assert!(lua
            .load("return bit32.btest(255, 15)")
            .eval::<bool>()
            .unwrap());
        assert!(!lua
            .load("return bit32.btest(240, 15)")
            .eval::<bool>()
            .unwrap());
    }

    #[test]
    fn string_pack_roundtrips_integers() {
        let lua = lua_with(&[
            Shim::StringPack,
            Shim::StringUnpack,
            Shim::StringPackHelpers,
        ]);
        assert_eq!(
            lua.load("return string.unpack('>i4', string.pack('>i4', 12345))")
                .eval::<f64>()
                .unwrap(),
            12345.0
        );
        assert_eq!(
            lua.load("return string.unpack('>i4', string.pack('>i4', -12345))")
                .eval::<f64>()
                .unwrap(),
            -12345.0
        );
        assert_eq!(
            lua.load("return string.unpack('<I2', string.pack('<I2', 65535))")
                .eval::<f64>()
                .unwrap(),
            65535.0
        );
        assert_eq!(
            lua.load("return string.unpack('i1', string.pack('i1', -1))")
                .eval::<f64>()
                .unwrap(),
            -1.0
        );
    }

    #[test]
    fn string_pack_roundtrips_strings() {
        let lua = lua_with(&[
            Shim::StringPack,
            Shim::StringUnpack,
            Shim::StringPackHelpers,
        ]);
        assert_eq!(
            lua.load("return string.unpack('>s4', string.pack('>s4', 'hello'))")
                .eval::<String>()
                .unwrap(),
            "hello"
        );
        assert_eq!(
            lua.load("return string.unpack('c3', string.pack('c3', 'abc'))")
                .eval::<String>()
                .unwrap(),
            "abc"
        );
        assert_eq!(
            lua.load("return string.unpack('z', string.pack('z', 'abc'))")
                .eval::<String>()
                .unwrap(),
            "abc"
        );
    }

    #[test]
    fn string_packsize_matches() {
        let lua = lua_with(&[Shim::StringPackSize, Shim::StringPackHelpers]);
        assert_eq!(
            lua.load("return string.packsize('>i4s4c3')")
                .eval::<f64>()
                .unwrap(),
            11.0
        );
        assert_eq!(
            lua.load("return string.packsize('bB')")
                .eval::<f64>()
                .unwrap(),
            2.0
        );
        assert_eq!(
            lua.load("return string.packsize('x x')")
                .eval::<f64>()
                .unwrap(),
            2.0
        );
    }

    #[test]
    fn utf8_char_and_codepoint() {
        let lua = lua_with(&[Shim::Utf8]);
        assert_eq!(
            lua.load("return utf8.char(72, 105)")
                .eval::<String>()
                .unwrap(),
            "Hi"
        );
        assert_eq!(
            lua.load("return utf8.char(955)").eval::<String>().unwrap(),
            "λ"
        );
        assert_eq!(
            lua.load("return utf8.codepoint('λ')")
                .eval::<f64>()
                .unwrap(),
            955.0
        );
        assert_eq!(
            lua.load("return utf8.codepoint('Hi', 2)")
                .eval::<f64>()
                .unwrap(),
            105.0
        );
    }

    #[test]
    fn utf8_len_and_offset() {
        let lua = lua_with(&[Shim::Utf8]);
        assert_eq!(
            lua.load("return utf8.len('hello')").eval::<f64>().unwrap(),
            5.0
        );
        assert_eq!(lua.load("return utf8.len('λ')").eval::<f64>().unwrap(), 1.0);
        assert_eq!(
            lua.load("return utf8.offset('hλllo', 2)")
                .eval::<f64>()
                .unwrap(),
            2.0
        );
        assert_eq!(
            lua.load("return utf8.offset('λh', 2)")
                .eval::<f64>()
                .unwrap(),
            3.0
        );
    }

    #[test]
    fn utf8_codes_iterates() {
        let lua = lua_with(&[Shim::Utf8]);
        let codepoints: String = lua
            .load("local out = {} for _, c in utf8.codes('Aλ') do out[#out + 1] = c end return table.concat(out, ',')")
            .eval()
            .unwrap();
        assert_eq!(codepoints, "65,955");
    }
}
