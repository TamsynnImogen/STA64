-- H09: weak tables would observe GC timing. The sandbox refuses `__mode` (and `__gc`) at
-- setmetatable; a `__mode` set on a metatable afterwards is ignored by the bundled Lua, so the
-- table stays strong. collectgarbage is removed (a stub answers).
local okw, errw = pcall(setmetatable, {}, { __mode = "k" })
local okg, errg = pcall(setmetatable, {}, { __gc = function() end })
local mt = {}
local K = setmetatable({}, mt)
mt.__mode = "k"
local mtv = {}
local W = setmetatable({}, mtv)
mtv.__mode = "v"
FINALISED = 0
local late = {}
setmetatable(late, {})
getmetatable(late).__gc = function() FINALISED = FINALISED + 1 end
local n = 0
MMM.register(function()
  n = n + 1
  for i = 1, 300 do
    W[n * 1000 + i] = { i }
    K[{}] = i
    local s = "str" .. n .. "_" .. i
  end
  local cw, ck = 0, 0
  for _ in pairs(W) do cw = cw + 1 end
  for _ in pairs(K) do ck = ck + 1 end
  obs(cw .. "/" .. ck .. " weak=" .. tostring(okw) .. ":" .. tostring(string.find(tostring(errw), "reproducible", 1, true) ~= nil)
    .. " gcguard=" .. tostring(okg) .. " fin=" .. FINALISED)
end)
