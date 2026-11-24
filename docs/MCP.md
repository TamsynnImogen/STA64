# Playing STA64 through MCP

STA64 can let a local MCP client take over one player slot in Instant Action or skirmish. Start the game with `--mcp-seat N`, where `N` is a zero-based player slot. For example, use `--mcp-seat 1` for the first computer opponent in a default two-player setup. That slot must be occupied when the match starts. The game's usual controller handles the seat until an MCP client claims it.

```sh
./sta64 --install-root /path/to/install --mcp-seat 1 \
  --mcp-endpoint "$HOME/.local/share/sta64/mcp/playtest.json"
```

If you omit `--mcp-endpoint`, the game writes `mcp/seat-N.json` in its user data directory. The descriptor contains a per-launch credential and the local port. Keep it private. STA64 removes it on clean exit. After a crash, choose a fresh path for the next launch or remove the old descriptor once that process has exited.

Configure your MCP client to launch the Python 3 server with the same descriptor path. Use absolute paths in the client configuration.

```json
{
  "mcpServers": {
    "sta64": {
      "command": "python3",
      "args": [
        "/path/to/STA64/tools/sta64_mcp.py",
        "--endpoint",
        "/home/example/.local/share/sta64/mcp/playtest.json"
      ]
    }
  }
}
```

The client talks to the Python server over stdio. The server connects to the game over the local loopback interface. [MCP stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)

## Playing a match

1. Start the match and call `status`. When the authorized seat is available, call `claim`.
2. Call `observe` for map bounds, the seat's resources and relations, own units and their build queues, and current sensor contacts. Supply the returned `next_after` as `after` to read the next page. A null cursor indicates the end.
3. Call `order` using IDs from the observation. A queued response includes a sequence number. Check `recent_orders` on a subsequent observation to see whether the simulation accepted it. Check the unit's position, cargo, or production queue for progress.
4. Call `release` to return the seat. Closing the MCP client also attempts release. If the client disappears, the game releases its seat after 90 seconds without a heartbeat. A new match requires a fresh claim.

For example, a move order takes `{"action":"move","units":[12],"position":[1500,0,2300]}`. Coordinates are in map space as `[x,y,z]`, with `y` as height. Use actual unit IDs and coordinates from the match.

| Action | Arguments |
| --- | --- |
| `move`, `move_warp` | `units`, `position`, optional `queue` |
| `attack`, `mine` | `units`, `target` |
| `stop`, `scout`, `search_and_destroy` | `units` |
| `build` | `builder`, `odf`, optional `position` for a construction site |
| `rally` | `units`, optional `target` or `position` |
| `special` | `units`, `slot`, optional `target` or `position` |
| `repair` | `units`, optional repair-yard `target` |

The game validates ownership, target visibility, and its normal order rules. Claiming a partner or opponent slot leaves your own controls available. Releasing restores that slot's prior controller and clears orders that are still waiting to enter the simulation. Orders already issued in the match can continue. MCP play is available in local matches. Entering the map editor ends the lease.
