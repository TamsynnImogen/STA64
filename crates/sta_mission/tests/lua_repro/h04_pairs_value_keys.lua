-- H04: keys whose hash derives from the value only (booleans, integers, floats), and the
-- border (#) of tables with holes. Expected reproducible.
local T = {}
T[true] = "t"
T[false] = "f"
for i = 1, 30 do
  T[i * 1000003] = i
  T[-i] = -i
  T[i + 0.5] = i .. "f"
  T[2 ^ 40 + i] = "b" .. i
end
local n = 0
MMM.register(function()
  n = n + 1
  local out = {}
  for k, v in pairs(T) do out[#out + 1] = tostring(k) end
  local U = {}
  for i = 1, 40 do
    if (i * 7 + n) % 3 ~= 0 then U[i] = i end
  end
  local V = {}
  for i = 40, 1, -1 do
    if (i * 5 + n) % 4 ~= 0 then V[i] = i end
  end
  obs(chk({ #out }) .. ":" .. join(out) .. " border " .. #U .. "," .. #V)
  T[n * 7919] = n
end)
