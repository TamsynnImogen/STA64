# Playing STA64 through MCP

In Instant Action, open the player dropdown and choose **MCP** for the slot you want the client to play. Keep your own slot set to **Player**, or choose **Observer** in the team menu to watch. Launch the match normally. The MCP slot waits for a client to claim it and goes idle again when released. One slot per match can be reserved for MCP.

The three AI options use the mounted mod's `Dynamic_Localized_Strings.h` captions when available. The English fallbacks are **AI Easy**, **AI Medium**, and **AI Hard**.

The menu creates `mcp/instant-action.json` in STA64's user data directory. Use `--mcp-endpoint` to specify a different path. The descriptor contains a per-launch credential and the local port. Keep it private. STA64 removes it on clean exit. After a crash, choose a fresh path for the next launch or remove the old descriptor once that process has exited.

For command-line skirmish, use `--mcp-seat N` to allow takeover of an already occupied, zero-based seat. The default descriptor in that mode is `mcp/seat-N.json`. A computer seat authorized this way uses its original AI whenever MCP control is released.

Configure your MCP client to launch the Python 3 server with the same descriptor path. Use absolute paths in the client configuration.

```json
{
  "mcpServers": {
    "sta64": {
      "command": "python3",
      "args": [
        "/path/to/STA64/tools/sta64_mcp.py",
        "--endpoint",
        "/home/example/.local/share/sta64/mcp/instant-action.json"
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

The game validates ownership, target visibility, and its normal order rules. Claiming a partner or opponent slot leaves your own controls available. Releasing clears orders that are still waiting to enter the simulation. Orders already issued in the match can continue. MCP play is available in local matches. Entering the map editor ends the lease.
