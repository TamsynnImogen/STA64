-- H09b: as H09 without address-keyed tables: a late weak-values mode under integer keys, with
-- string garbage driving the collector. The table stays strong, so the count only grows.
local mt = {}
local W = setmetatable({}, mt)
mt.__mode = "v"
local n = 0
MMM.register(function()
  n = n + 1
  for i = 1, 300 do
    W[n * 1000 + i] = { i }
    local s = "str" .. n .. "_" .. i
  end
  local cw = 0
  for _ in pairs(W) do cw = cw + 1 end
  obs(cw)
end)
