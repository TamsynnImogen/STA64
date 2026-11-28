-- Prepended to every hazard script. One obs() call per tick = one LOG line.
LOG = {}
function obs(s) LOG[#LOG + 1] = s end
function join(t, sep) return table.concat(t, sep or ",") end
-- order-sensitive checksum of a list of numbers
function chk(list)
  local h = 7
  for i = 1, #list do h = (h * 31 + list[i]) % 1000000007 end
  return h
end
