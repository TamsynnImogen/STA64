-- H11: float formatting and number conversions (same machine: libc printf and strtod).
local n = 0
MMM.register(function()
  n = n + 1
  local x = n / 7
  local vals = { 0.1 + 0.2, 1 / 3, 100 / 3, 1e15, 1e100, 2 ^ 53, -0.0, 0 / 0, 1 / 0, -1 / 0, x, x * 1e10, 2 ^ 0.5, 7 // 2, 7.5 // 2, 3 % -2, -3 % 2, 5.5 % -2 }
  local out = {}
  for i, v in ipairs(vals) do
    out[#out + 1] = tostring(v) .. "|" .. string.format("%.17g|%g|%5.2f|%a", v, v, v, v)
  end
  out[#out + 1] = tostring(tonumber("0x10")) .. tostring(tonumber("1e2")) .. tostring(tonumber(" 12 ")) .. tostring(math.floor(-0.5)) .. tostring(math.tointeger and math.tointeger(3.0))
  obs(join(out, " "))
end)
