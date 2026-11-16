# Lobby and spectator sessions

**Status: APPROVED (partial).** The state machine and wire messages are
implemented; UDP relay wiring is host-provided.

## 1. Lobby

`manato_net::lobby::LobbyState` tracks:

- `host` member id and `max_players` (spectators do not consume player slots).
- `members` with `LobbyRole::{Host, Guest, Spectator}` and a ready flag.
- `phase` — `Open`, `Starting`, `InMatch`, `Closed`.

Operations:

- `join(name, spectator)` — players are rejected when slots are full; at most
  32 total connections.
- `leave(id)` — removes a member; if the host leaves, the first remaining
  player becomes host (spectators cannot host); an empty lobby closes.
- `set_ready(id, ready)` — players only, `Open` phase only.
- `start(id)` — host only, at least two players, all players ready; moves to
  `Starting`, then `enter_match()` sets `InMatch`.

## 2. Wire messages

`LobbyMessage` is serialized with `bincode`:

- `Join { name, spectator }`
- `Leave { id }`
- `Ready { id, ready }`
- `Start { id }`
- `Frame { frame, checksum }` — relayed rollback frame metadata for spectators.

`encode()`/`decode()` are transport-agnostic; a relay can carry them over UDP
or any other channel.

## 3. Spectators

`SpectatorSync` accepts increasing rollback frame numbers, rejects stale or
duplicate frames, counts gaps as `missed_frames`, and reports `lagging()` once
more than 60 frames (one second) are missing.

## 4. Not modeled

- UDP/TCP relay sockets and lobby discovery, NAT traversal, and matchmaking.
- Spectator state streaming (only frame/checksum metadata is defined).
- Chat/moderation, spectator input isolation beyond the role flags.
