-- H13: the order of the collection a host function returns (EntityFinder():find()), with a
-- world view whose unit order rotates every tick.
MMM.register(function()
  local list = EntityFinder():find()
  local ids = {}
  for i, e in ipairs(list) do ids[i] = tostring(e.id) end
  obs(#list .. ":" .. join(ids))
end)
