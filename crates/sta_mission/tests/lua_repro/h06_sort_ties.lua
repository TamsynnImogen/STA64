-- H06: table.sort with a comparator that leaves ties (compares key only), at several sizes.
-- Stock Lua 5.3's quicksort switched to a clock()/time() based pivot after a badly unbalanced
-- partition on a range over 100 elements; the "adv" inputs are built so that the first pivot is
-- the minimum, which triggered that switch. The sandbox's sort is a stable merge sort.
-- Also comparators that are not a strict weak order (no error, some fixed permutation).
local function items(n, mod, adv)
  local a = {}
  for i = 1, n do a[i] = { id = i, key = i % mod + 1 } end
  if adv then
    a[1].key = 0
    a[(n + 1) // 2].key = 0
    a[n].key = 1000
  end
  return a
end
local function sorted_checksum(n, mod, cmp, adv)
  local a = items(n, mod, adv)
  local ok, err = pcall(table.sort, a, cmp)
  if not ok then return "err" end
  local ids = {}
  for i = 1, #a do ids[i] = a[i].id end
  return chk(ids)
end
MMM.register(function()
  local out = {}
  local by_key = function(x, y) return x.key < y.key end
  for _, spec in ipairs({ { 20, 3 }, { 50, 5 }, { 100, 7 }, { 150, 7 }, { 300, 4 }, { 1000, 3 } }) do
    out[#out + 1] = "n" .. spec[1] .. "=" .. sorted_checksum(spec[1], spec[2], by_key, false)
  end
  for _, spec in ipairs({ { 60, 4 }, { 300, 4 }, { 1000, 3 } }) do
    out[#out + 1] = "adv" .. spec[1] .. "=" .. sorted_checksum(spec[1], spec[2], by_key, true)
  end
  out[#out + 1] = "bad150=" .. sorted_checksum(150, 7, function() return true end, false)
  out[#out + 1] = "le150=" .. sorted_checksum(150, 7, function(x, y) return x.key <= y.key end, false)
  obs(join(out, " "))
end)
