# Timer bridge

The clean-room Recovery port uses `Timer(name, seconds, Timer.State.Started)`
and `:hook(hookName, nil, callback, nil)`. Those shapes are supported. The
inventory documents other names without signatures; their behavior here is
an explicit bridge convention, not verified native MMM parity.

Timers use the mission view's integer tick. Finite nonnegative seconds round
half-up to at least one tick. The constructor's optional state defaults to
Stopped; timers are one-shot until `repeats=true`. `start()` restarts the
whole interval from the current tick; `stop()` clears the deadline. Changing
`interval` reschedules an active timer. Overflow returns a Lua error before
state changes. No wall clock or background thread is involved.

`Timer.find(name)` returns a handle or nil; `getAll()` returns handles in
registration order. Duplicate timer/hook names return errors. `stopAll()`
stops every timer. `unhook(name)` removes one hook; `unhook()` removes all.
`id`, `interval`, `repeats`, `state` and hook count `hooks` are readable;
`interval` and `repeats` are writable.

The callback receives the timer handle and a hook object with `argument` (the
fourth hook argument, or the hook name when that argument is nil, kept from
the earlier bridge) and `id`, as installed MMM scripts read them; a table
argument's fields also stay reachable on the hook object. Non-nil conditions are rejected until
their contract is established. Due callbacks run in registration order, with
no context borrow held during Lua calls. Restarting/stopping/unhooking can
invalidate later firings queued for that timer in the same pass. At most one
expiration per timer is processed per controller step, even after a tick jump.
Each retained hook is disabled after five errors, including errors after a
restart; a removed/replaced hook does not inherit another hook's error count.

The runtime processes condition monitors, then timers, then registered mission
callbacks. Timers created by an object's first `setup` therefore start from
that first controller tick and cannot fire earlier than the next tick.
