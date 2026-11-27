//! The Lua text the converted scripts start with, as verbatim constants (LUA-15c).
//!
//! `PRELUDE` and `ENGINE` are the `PRELUDE` and `ENGINE` raw strings of `tools/dsl_to_lua.py` (the
//! engine's rule and script scheduling written for the converted script: read from the
//! Armada II executable, static), `A1_PRELUDE` is the one of `tools/a1_to_lua.py`. They are
//! the converter's own Lua, not game content. Every constant starts with a newline, as the
//! Python raw strings do. The tests compare them with the Python tools' constants.

use crate::{DecodeError, Result};

/// `PRELUDE` of `tools/dsl_to_lua.py`: helper functions, the stand-in engine glue.
pub const PRELUDE: &str = r#"
-- ---------------------------------------------------------------- runtime
-- The engine's rule and script scheduling, written for the converted script
-- (read from the Armada II executable, static; see tools/dsl_to_lua.py).

local function T(v) return v ~= nil and v ~= false and v ~= 0 end
local function N(v) if v == true then return 1 elseif type(v) ~= 'number' then return 0 end return v end
local function I(v) v = N(v) if v >= 0 then return math.floor(v) end return -math.floor(-v) end
local function I32(v) v = N(v) if math.type(v) == 'float' then return v end v = v & 0xffffffff if v >= 0x80000000 then v = v - 0x100000000 end return v end
local function FTOL(v) return I(v) end
local function SAR(x, n) x = I(x) n = I(n) if x >= 0 then return x >> n end return ~((~x) >> n) end
local function IDIV(x, y) x, y = I(x), I(y) if y == 0 then return 0 end local q = x // y if q < 0 and q * y ~= x then q = q + 1 end return q end
local function IMOD(x, y) x, y = I(x), I(y) if y == 0 then return 0 end return x - IDIV(x, y) * y end
local function VEC(v, k) if type(v) ~= 'table' and type(v) ~= 'userdata' then return 0 end local ok, r = pcall(function() return ({v.x, v.y, v.z})[k] end) return ok and r or 0 end
-- A stack slot read as a float: a constant the compiler stored as raw float bits is an
-- integer here, anything the code computed is already a float.
local function FB(v)
    if math.type(v) == 'integer' then return (string.unpack('<f', string.pack('<I4', v & 0xffffffff))) end
    return N(v)
end
local function STRCMP(a, b)
    a, b = tostring(a or ''), tostring(b or '')
    if a == b then return 0 end
    return a < b and -1 or 1
end
-- sprintf into a buffer (the buffer becomes the formatted string); `%i` and `%u` are `%d`.
local function SPRINTF(fmt, ...)
    -- a byte scan, not a pattern: the mission sandbox limits pattern size and quantifiers
    local args, n, out, i = { ... }, 0, {}, 1
    fmt = tostring(fmt or '')
    while i <= #fmt do
        local c = fmt:sub(i, i)
        if c ~= '%' then
            out[#out + 1] = c
            i = i + 1
        else
            local j = i + 1
            while j <= #fmt and ('-+ 0123456789.'):find(fmt:sub(j, j), 1, true) do j = j + 1 end
            local flags = fmt:sub(i + 1, j - 1)
            while j <= #fmt and ('hlLIw#'):find(fmt:sub(j, j), 1, true) do j = j + 1 end  -- length modifiers
            local conv = fmt:sub(j, j)
            if conv == '' then break end
            i = j + 1
            if conv == '%' then
                out[#out + 1] = '%'
            else
                n = n + 1
                local v = args[n]
                if conv == 's' then out[#out + 1] = string.format('%' .. flags .. 's', tostring(v or ''))
                elseif conv == 'c' then out[#out + 1] = string.char(I(v) & 0xff)
                elseif conv == 'u' then out[#out + 1] = string.format('%' .. flags .. 'd', I(v) & 0xffffffff)
                elseif conv == 'i' or conv == 'd' then out[#out + 1] = string.format('%' .. flags .. 'd', I(v))
                else out[#out + 1] = string.format('%' .. flags .. conv, I(v) & 0xffffffff) end
            end
        end
    end
    return table.concat(out)
end
local function U(x) return I(x) & 0xffffffff end
local function ULT(x, y) return math.ult(I(x) & 0xffffffff, I(y) & 0xffffffff) end

local Mission = Mission_
local M = {}
local V, S, O, G = Mission.vars, Mission.state, Mission.objects, Mission.globals

local ENGINE_WRITES = {}
-- Variables the engine writes (team strength, moons, warp attempts, the cineractive flags):
-- the script only reads them. A read goes through `S`'s __index, is counted in
-- Mission.engineReads[name], and asks the host (`A2.EngineVariable(name)`, bound later) for the
-- value; with no host answer it reads the engine's start value.
setmetatable(S, { __index = function(_, p)
    local var = Mission.engineWritten[p]
    if not var then return nil end
    Mission.engineReads[var.name] = (Mission.engineReads[var.name] or 0) + 1
    local f = A2 and A2.EngineVariable
    if type(f) == 'function' then
        local ok, v = pcall(f, var.name)
        if ok and type(v) == 'number' then Mission.setEngineValue(var, v) end
    end
    return var.value
end })
-- The calls that did not work, in order of first occurrence (for the report).
local function problem(name, kind, why)
    Mission.lastProblem = name
    if not Mission.problems[name] then
        Mission.problems[name] = kind
        if #Mission.problemOrder < 60 then
            Mission.problemOrder[#Mission.problemOrder + 1] = { name, kind, Mission.time or 0, why }
        end
    end
end
-- A handle the mission never set is -1: no object. The engine's object guard
-- (handle to object, null when absent) makes such a call do nothing and return
-- 0 (read from the executable, static, for the setters read; assumed for the
-- rest), where an A2 binding insists on an unsigned handle and raises. That
-- raise is answered 0 here and counted in Mission.invalid, not as a failure.
-- The same guard covers a handle whose object has died or been removed (the
-- binding raises "object N is not alive"): missions keep polling the handles of
-- destroyed units, and a raise would block the whole rule (LUA-17; assumed for
-- the queries, not measured).
local function negative(...)
    for i = 1, select('#', ...) do
        local v = select(i, ...)
        if math.type(v) == 'integer' and v < 0 then return true end
    end
    return false
end
local function A(name, ...)
    local f = A2 and A2[name]
    Mission.calls[name] = (Mission.calls[name] or 0) + 1
    if ENGINE_WRITES[name] then ENGINE_WRITES[name]() end
    if type(f) ~= 'function' then
        Mission.missing[name] = (Mission.missing[name] or 0) + 1
        Mission.callFailed = true
        problem(name, 'unbound')
        if A2 and A2.NoteGap then A2.NoteGap(name, ...) end
        return nil
    end
    local ok, v = pcall(f, ...)
    if not ok then
        if (tostring(v):find('unsigned integer', 1, true) and negative(...))
            or tostring(v):find(' is not alive', 1, true) then
            Mission.invalid[name] = (Mission.invalid[name] or 0) + 1
            return 0
        end
        Mission.failed[name] = (Mission.failed[name] or 0) + 1
        Mission.lastError = tostring(v)
        Mission.callFailed = true
        problem(name, 'raised', tostring(v))
        return nil
    end
    if name == 'GetHandle' then
        if v == 0 then
            -- a label no object of the map carries: the invalid handle
            Mission.unresolved[(...)] = true
        else
            Mission.resolved = (Mission.resolved or 0) + 1
        end
    end
    return v
end

-- A call with an `int &` out parameter (RecruitSpecialForces): the binding returns `ok, value`;
-- the value is written to the slot the script passed (OUTV), the slot keeps its value when
-- the call did not run.
local function AO(name, ...)
    Mission.out = nil
    local f = A2 and A2[name]
    if type(f) == 'function' then
        local ok, v, o = pcall(f, ...)
        if ok then
            Mission.calls[name] = (Mission.calls[name] or 0) + 1
            Mission.out = o
            return v
        end
    end
    return A(name, ...)
end
local function OUTV(cur)
    local o = Mission.out
    Mission.out = nil
    if o == nil then return cur end
    return o
end

-- std::vector<int> of the DLL's C++ code (MSVC 6 layout): a table with the element count `n`;
-- an iterator or element reference is a byte offset from begin(), as the compiled pointer
-- arithmetic has it. A zeroed static is an empty vector.
local function VG(k) local v = G[k] if type(v) ~= 'table' then v = { n = 0 } G[k] = v end return v end
local function VL(L, k) local v = L[k] if type(v) ~= 'table' then v = { n = 0 } L[k] = v end return v end
local function VCTOR(v) for i = 1, v.n do v[i] = nil end v.n = 0 end
local VCLEAR, VDTOR = VCTOR, VCTOR
local function VBEGIN(v) return 0 end
local function VEND(v) return 4 * v.n end
local function VSIZE(v) return v.n end
local function VINDEX(v, i) return 4 * I(i) end
local function VPUSH_BACK(v, x) v.n = v.n + 1 v[v.n] = x end
local function VERASE(v, it) local i = I(it) // 4 + 1 if i >= 1 and i <= v.n then for j = i, v.n - 1 do v[j] = v[j + 1] end v[v.n] = nil v.n = v.n - 1 end return it end
local function VGET(v, off) local x = v[I(off) // 4 + 1] if x == nil then return 0 end return x end

-- Methods of engine objects a ScriptInterface call returned (fleets).
local function FM(obj, cls, name, ...)
    local key = cls .. '.' .. name
    Mission.calls[key] = (Mission.calls[key] or 0) + 1
    local f = type(obj) == 'table' and obj[name] or nil
    if type(f) ~= 'function' then
        Mission.missing[key] = (Mission.missing[key] or 0) + 1
        Mission.callFailed = true
        problem(key, 'unbound')
        if A2 and A2.NoteGap then A2.NoteGap(key, ...) end
        return nil
    end
    local ok, v = pcall(f, obj, ...)
    if not ok then
        Mission.failed[key] = (Mission.failed[key] or 0) + 1
        Mission.callFailed = true
        problem(key, 'raised', tostring(v))
        return nil
    end
    return v
end

function M.notDecoded(name, why)
    Mission.notDecoded[name] = (Mission.notDecoded[name] or 0) + 1
    Mission.ran[#Mission.ran + 1] = name .. ' (not decoded)'
end

-- Variables (hot_int, hot_float, dirty_int, timer_int). A write that changes
-- the value marks the variable dirty and puts it on the affected list; once a
-- tick, before the rules run, each affected variable wakes the rules that
-- depend on it and is set clean. A dirty_int never becomes clean (the
-- engine's ALWAYS_EVALUATE is one), so its rules are evaluated every tick.
-- A timer reads 1 when it runs out; setting it clean turns 1 into 2 and any
-- other non-zero value into 0 and keeps it affected, so a rule reading the
-- timer sees it true for exactly one evaluation.
local START = { hot_int = -1, hot_float = -1.0, dirty_int = -1, timer_int = 0 }
local function addAffected(var)
    if not var.listed then
        var.listed = true
        Mission.affected[#Mission.affected + 1] = var
    end
end
local function NewVar(kind, name)
    local var = { kind = kind, name = name, value = START[kind] or 0, deps = {}, remaining = 0, dirty = false }
    if kind == 'dirty_int' then var.dirty = true addAffected(var) end
    return var
end
-- Variables the engine owns (ALWAYS_EVALUATE, isCineractiveCancel/Done and
-- the per-team AI values): one per game, with the engine's start values; the
-- engine's writes to them are pushed by the host (cineractive flags, team strength).
local function EngineVar(name, kind)
    local var = Mission.engineVars[name]
    if not var then
        var = NewVar(kind or 'hot_int', name)
        var.engine = true
        -- The AI's team strengths read 0.0 from the mission's creation (-1.0 is the engine's
        -- value before any mission exists, never visible during one). Armada II only: the Armada I
        -- writer is not established, its variables keep the start value.
        if Mission.game ~= 'a1' and name:match('^strength_team_%d$') then var.value = 0.0 end
        Mission.engineVars[name] = var
    end
    return var
end
local function BindVar(p, kind, var)
    if type(var) ~= 'table' then var = NewVar(kind, p) end
    var.p = p
    V[p] = var
    if var.engine and p ~= 'p_ALWAYS_EVALUATE' then
        -- engine-written: read through the counted __index of S, never a stored copy
        Mission.engineWritten[p] = var
        rawset(S, p, nil)
    else
        S[p] = var.value
    end
end
local function varOf(p)
    local var = V[p]
    if not var then var = NewVar('hot_int', p) BindVar(p, 'hot_int', var) end
    return var
end
local function setValue(var, value)
    if var.value ~= value then
        var.value = value
        if not Mission.engineWritten[var.p] then S[var.p] = value end
        if not var.dirty then
            var.dirty = true
            addAffected(var)
        end
    end
end
function Mission.setEngineValue(var, v) setValue(var, v) end
local function stopTimer(var)
    if var.remaining > 0 then
        for i, x in ipairs(Mission.timers) do
            if x == var then table.remove(Mission.timers, i) break end
        end
    end
    var.remaining = 0
end
local function VarOp(p, op, x)
    local var = varOf(p)
    if op == 'set' then
        if var.kind ~= 'hot_float' then x = I(x) else x = N(x) end
        setValue(var, x)
    elseif op == 'start' then
        x = N(x)
        if x < 0.001 then
            stopTimer(var)
            setValue(var, 1)
        else
            if var.remaining < 0.001 then Mission.timers[#Mission.timers + 1] = var end
            var.remaining = x
        end
    elseif op == 'stop' then
        stopTimer(var)
    elseif op == 'mark_dirty' then
        if not var.dirty then var.dirty = true addAffected(var) end
    elseif op == 'set_clean' then
        var.dirty = var.kind == 'dirty_int'
    end
end
-- While a cineractive runs, the engine's cineractive update writes
-- isCineractiveDone (its shots finished) and isCineractiveCancel (every
-- player skipped) each frame; with no skip input and shots running both read
-- 0 from the first frame (read from the executable, static).
ENGINE_WRITES.CineractiveStart = function()
    for _, n in ipairs({ 'isCineractiveCancel', 'isCineractiveDone' }) do
        local var = Mission.engineVars[n]
        if var then setValue(var, 0) end
    end
end

local function VarQuery(p, q)
    local var = varOf(p)
    if q == 'time' then return var.remaining end
    if q == 'is_dirty' then return (var.dirty or var.kind == 'dirty_int') and 1 or 0 end
    return 0
end

-- Scripts: an action queues a named script on the world (same names run side
-- by side); scripts run after the rules, oldest first, one step per tick each;
-- a step's return value is not used; a script whose steps have all run is
-- removed (one pass after its last step, once it holds no reference, as the
-- engine deletes it). Steps that wait on the engine (fades, cineractives, audio, movies)
-- are held by `A2.ScriptWaits` (see runScripts).
local function NewScript(name)
    local s = { name = name, steps = {}, names = {}, at = 1 }
    Mission.scripts[#Mission.scripts + 1] = s
    return s
end
local function AddStep(s, fn, name)
    if type(s) ~= 'table' then return end
    s.steps[#s.steps + 1] = fn
    s.names[#s.names + 1] = name
end
local function TrashRule(name)
    for _, rule in ipairs(Mission.rules) do
        if rule.name == name then rule.trashed = true end
    end
    for i = #Mission.satisfied, 1, -1 do
        if Mission.satisfied[i].trashed then table.remove(Mission.satisfied, i) end
    end
end
"#;

/// `ENGINE` of `tools/dsl_to_lua.py`: one engine tick (timers, affected variables, rules, scripts).
pub const ENGINE: &str = r#"
-- ---------------------------------------------------------------- engine
-- One engine tick: timers, affected variables, rules, scripts.
-- The step reference count of a script (read from the executable, static):
-- a blocking engine call (FadeIn/FadeOut, CineractiveStart, AudioMessage,
-- PlayMusic, PlayCinematicMovie, WaitForEnterKey) given the script and
-- "don't wait" false holds one reference until it finishes; a script with a
-- reference does not run its next step and is not removed. `A2.ScriptWaits`
-- reports the count (the client reports completion; see the stand-in host).
local function held(s)
    if not (A2 and A2.ScriptWaits) then return 0 end
    local ok, n = pcall(A2.ScriptWaits, s)
    return ok and tonumber(n) or 0
end

function Mission:runScripts()
    local i = 1
    while i <= #self.scripts do
        local s = self.scripts[i]
        local fn = s.steps[s.at]
        local waits = held(s)
        if not fn then
            -- every step has run: the engine deletes the script once it holds no reference
            if waits == 0 then table.remove(self.scripts, i) else i = i + 1 end
        elseif waits > 0 then
            self.waited = (self.waited or 0) + 1
            i = i + 1
        else
            local name = s.names[s.at]
            s.at = s.at + 1
            self.stepCount = self.stepCount + 1
            if #self.ran < 2000 then self.ran[#self.ran + 1] = name end
            self.lastStep = name
            local ok, err = pcall(fn, s)
            if not ok then self.stepErrors[#self.stepErrors + 1] = name .. ': ' .. tostring(err) end
            i = i + 1
        end
    end
end

function Mission:runRules()
    -- affected variables wake their rules, then are set clean
    local woken, order = {}, {}
    local list = self.affected
    self.affected = {}
    for _, var in ipairs(list) do
        var.listed = false
        for _, rule in ipairs(var.deps) do
            if not woken[rule] then woken[rule] = true order[#order + 1] = rule end
        end
        if var.kind == 'timer_int' then
            if var.value == 1 then var.value = 2 elseif var.value ~= 0 then var.value = 0 else var.dirty = false end
            if not Mission.engineWritten[var.p] then S[var.p] = var.value end
        elseif var.kind ~= 'dirty_int' then
            var.dirty = false
        end
        if var.dirty or var.kind == 'dirty_int' then addAffected(var) end
    end
    -- a woken rule enters or leaves the satisfied list when its result changes
    for _, rule in ipairs(order) do
        if not rule.trashed then
            self.callFailed = false
            local ok, holds = pcall(rule.when, S, O)
            local r = (ok and holds == true and not self.callFailed) and 1 or 0
            if not ok then self.conditionErrors[rule.name] = tostring(holds) end
            if self.callFailed and not self.blocked[rule.name] then self.blocked[rule.name] = self.lastProblem end
            if r ~= rule.last then
                rule.last = r
                if r == 1 then
                    table.insert(self.satisfied, 1, rule)
                    self.fired[#self.fired + 1] = rule.name
                    self.lastRule = rule.name
                else
                    for i, x in ipairs(self.satisfied) do
                        if x == rule then table.remove(self.satisfied, i) break end
                    end
                end
            end
        end
    end
    -- every satisfied rule fires every tick, newest first, until a later
    -- evaluation finds it false
    local fire = {}
    for i, rule in ipairs(self.satisfied) do fire[i] = rule end
    for _, rule in ipairs(fire) do
        if not rule.trashed then
            self.fireCount[rule.name] = (self.fireCount[rule.name] or 0) + 1
            local action = M[rule.action]
            if action then
                local ok, err = pcall(action)
                if not ok then self.stepErrors[#self.stepErrors + 1] = rule.action .. ': ' .. tostring(err) end
            else
                self.notDecoded[rule.action] = (self.notDecoded[rule.action] or 0) + 1
            end
        end
    end
end

function Mission:setup()
    -- load order of the engine: ScriptExtendInterface, the rule DLL's
    -- registration, then ScriptInit before the first tick
    if M.ScriptExtendInterface then M.ScriptExtendInterface(SI, 0x33) end
    for _, rule in ipairs(self.rules) do
        rule.last = 0
        for _, p in ipairs(rule.deps) do
            local var = varOf(p)
            var.deps[#var.deps + 1] = rule
        end
    end
    if M.ScriptInit then M.ScriptInit() end
end

function Mission:update(time)
    local dt = time - (self.time or time)
    self.time = time
    local running = {}
    for i, var in ipairs(self.timers) do running[i] = var end
    for _, var in ipairs(running) do
        if var.remaining >= 0.001 then
            var.remaining = var.remaining - dt
            if var.remaining < 0.001 then
                setValue(var, 1)
                for i, x in ipairs(self.timers) do
                    if x == var then table.remove(self.timers, i) break end
                end
            end
        end
    end
    self:runRules()
    self:runScripts()
end

MMM.register(Mission)
"#;

/// `A1_PRELUDE` of `tools/a1_to_lua.py`: the Armada I additions (undecoded calls are counted).
pub const A1_PRELUDE: &str = r#"
-- Armada I: a call or a function the converter could not decode is counted
-- here when it runs (never dropped silently): Mission.undecoded[call].
local function UD(name, why)
    Mission.undecoded[name] = (Mission.undecoded[name] or 0) + 1
    Mission.callFailed = true
    problem(name, 'undecoded', why)
    return nil
end
function M.notDecoded(name, why, calls)
    Mission.notDecoded[name] = (Mission.notDecoded[name] or 0) + 1
    Mission.ran[#Mission.ran + 1] = name .. ' (not decoded)'
    problem(name, 'not decoded', why)
    for _, c in ipairs(calls or {}) do
        Mission.undecoded[c] = (Mission.undecoded[c] or 0) + 1
    end
end
"#;

/// The text `a1_prelude` replaces in [`PRELUDE`] and what it puts there.
const A1_REPLACEMENTS: [(&str, &str); 1] = [("local f = A2 and A2[name]", "local f = A1 and A1[name]")];

/// `a1_prelude()` of `tools/a1_to_lua.py`: [`PRELUDE`] with the engine table `A2` replaced by
/// `A1` (every occurrence of the replaced text). A [`DecodeError`] (kind `Decode`) when
/// the text to replace is not in the shared prelude any more.
pub fn a1_prelude() -> Result<String> {
    let mut src = PRELUDE.to_owned();
    for (old, new) in A1_REPLACEMENTS {
        if !src.contains(old) {
            return Err(DecodeError::new(format!("tools/dsl_to_lua.py prelude changed: `{old}` not found")));
        }
        src = src.replace(old, new);
    }
    Ok(src)
}

/// The replaced text, so a test can check the target still exists in [`PRELUDE`].
#[doc(hidden)]
pub fn a1_replacement_targets() -> Vec<&'static str> {
    A1_REPLACEMENTS.iter().map(|(old, _)| *old).collect()
}
