# Delphi

A Bitcoin price oracle that reads the price from the chain itself. The daemon fetches blocks and
unconfirmed transactions from Bitcoin peers over P2P, validates them, finds outputs sized at round
dollar amounts, and estimates the BTC/USD price from them. It signs every result and publishes it as
a Nostr note. The dashboard at <https://delphi.openlabs.lat> reads those notes.

## Integration

You can read the feeds straight from the relay with any Nostr client. No API key is needed.

| | |
|---|---|
| Relay | `wss://rof.illuminodes.com` |
| Publisher | `npub1hj2vnmtqepcwpwpxh49dlqulve6qf4d9u0v99shqluzuwcy7n0sql0cvwv` |
| Publisher (hex) | `bc94c9ed60c870e0b826bd4adf839f667404d5a5e3d852c2e0ff05c7609e9be0` |

Always filter on the publisher key and check each note's signature. Anyone can post notes of the
same kinds to the relay.

### Kinds

| Kind | Stored by the relay | Contents |
|---|---|---|
| `8078` | yes, permanently | Audits: the final chain price for one window at one block |
| `20078` | no, live only | Mempool price, fits, recent blocks and daemon status |
| `20079` | no, live only | One note per unconfirmed transaction the oracle judged |
| `30078` | yes, newest per `d` tag | The latest `status`, `blocks`, `fit-chain-1h` and `fit-chain-1d` |

Every note carries a `w` tag naming its window. Filter on it with `#w`.

| Window | Kind | Updated |
|---|---|---|
| `chain-1h` | `8078` | every block, price over the last 6 blocks |
| `chain-1d` | `8078` | every block, price over the last 144 blocks |
| `mempool` | `20078` | every few seconds, from unconfirmed transactions |
| `fit-mempool`, `fit-chain-1h`, `fit-chain-1d` | `20078` | the evidence behind each price |
| `blocks` | `20078` | the newest blocks with their statistics |
| `status` | `20078` | daemon state: tip, settled height, anchor price |
| `tx` | `20079` | one transaction, with verdict tag `v` and stage tag `s` |

The stored copies (kind `30078`) use the `d` tag `latest-<window>`, for example `latest-status`,
and keep the original window in `w`.

### Audits

An audit is published once its block is at least 6 deep and enough peer groups agree on it. Its
`created_at` is the block's timestamp and its `h` tag is the block height.

```json
{
  "window": "chain-1h",
  "blocks": 6,
  "price": 80455.27,
  "deviation": 0.163431,
  "mad_usd": 2629.78,
  "samples": 4437,
  "start_height": 973566,
  "stop_height": 973571,
  "start_time": 1793197416,
  "stop_time": 1793200800,
  "stop_hash": "00000000000000000000ddd1fd4ba1476098dfb52a15c00174622cd4587a64b3",
  "algorithm": { "name": "delphi-oracle", "version": 1, "...": "..." }
}
```

If a reorg replaces the block an audit was priced at, the oracle publishes a new audit for that
height with an `e` tag holding the old note's id. Drop the note the `e` tag names.

### Mempool price

```json
{
  "window": "mempool",
  "source": "mempool",
  "price": 80412.50,
  "samples": 412,
  "transactions": 206,
  "band_low": 69960.23,
  "band_high": 92524.56,
  "anchor_age_blocks": 2
}
```

The mempool search is limited to `band_low`..`band_high`, a band around the latest hourly chain
price. `anchor_age_blocks` is how many blocks old that chain price is.

### Requests

Audit history for one window over the last day:

```json
["REQ", "history", {
  "kinds": [8078],
  "authors": ["bc94c9ed60c870e0b826bd4adf839f667404d5a5e3d852c2e0ff05c7609e9be0"],
  "#w": ["chain-1d"],
  "since": 1793114400
}]
```

The live mempool price:

```json
["REQ", "live", {
  "kinds": [20078],
  "authors": ["bc94c9ed60c870e0b826bd4adf839f667404d5a5e3d852c2e0ff05c7609e9be0"],
  "#w": ["mempool"]
}]
```

The daemon's current state, without waiting for the next live note:

```json
["REQ", "state", {
  "kinds": [30078],
  "authors": ["bc94c9ed60c870e0b826bd4adf839f667404d5a5e3d852c2e0ff05c7609e9be0"],
  "#d": ["latest-status"]
}]
```

The same history request with [nak](https://github.com/fiatjaf/nak):

```sh
nak req -k 8078 -a bc94c9ed60c870e0b826bd4adf839f667404d5a5e3d852c2e0ff05c7609e9be0 \
  -t w=chain-1d -l 24 wss://rof.illuminodes.com
```
