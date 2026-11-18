-- Armada I, Federation 2: a player who defends. The fleet attacks the nearest enemy it can see;
-- the plan notes each enemy unit when first seen ("first") and when it has left that point ("acted").
local first, acted = {}, {}
local function armed()
  local o = {}
  for _, u in ipairs(units{mine = true}) do
    if u.maxhull > 0 and not u.building and not u.odf:find("const") and not u.odf:find("base") then o[#o + 1] = u end
  end
  return o
end
wait_until(control, 300)
while true do
  local foes = units{enemy = true}
  for _, f in ipairs(foes) do
    if not first[f.id] then
      first[f.id] = { x = f.x, z = f.z }
      note("first " .. f.id)
    end
    local p = first[f.id]
    if not acted[f.id] and math.abs(f.x - p.x) + math.abs(f.z - p.z) > 100 then
      acted[f.id] = true
      note("acted " .. f.id)
    end
  end
  for _, s in ipairs(armed()) do
    local f = nearest(s, foes)
    if f then attack(s, f) end
  end
  wait(10)
end
