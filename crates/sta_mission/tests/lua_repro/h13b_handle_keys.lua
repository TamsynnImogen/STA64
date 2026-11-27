-- H13b: entity handles (userdata created fresh by the host on every query) used as table keys
-- and walked with pairs.
MMM.register(function()
  local T = {}
  for _, e in ipairs(EntityFinder():find()) do T[e] = e.id end
  local order = {}
  for _, id in pairs(T) do order[#order + 1] = tostring(id) end
  obs(join(order))
end)
