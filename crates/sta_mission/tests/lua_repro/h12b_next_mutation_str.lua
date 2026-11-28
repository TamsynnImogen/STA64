-- H12b: as H12 but string keys (order depends on the string hash seed); only deleting existing
-- keys and assigning existing keys during traversal, which Lua allows.
local n = 0
MMM.register(function()
  n = n + 1
  local T = {}
  for i = 1, 30 do T["s" .. i] = i end
  local out = {}
  for k, v in pairs(T) do
    out[#out + 1] = k
    if v % 2 == 0 then T[k] = nil else T[k] = v + n end
  end
  local sum = 0
  for _, v in pairs(T) do sum = sum + v end
  obs(join(out) .. " sum=" .. sum)
end)
