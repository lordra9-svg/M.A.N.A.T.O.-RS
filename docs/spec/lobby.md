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
- `Welcome { id, host }` — server-assigned member id.
- `State { host, phase, members }` — full lobby snapshot broadcast on changes.
- `Snapshot { frame, checksum, index, total, data }` — one chunk of a host
  rollback state snapshot, relayed to spectators.

`encode()`/`decode()` are transport-agnostic; a relay can carry them over UDP
or any other channel.

## 3. Spectators

`LobbyServer` maps member ids to peer addresses and handles join/leave/ready/
start/frame messages, returning `(addr, message)` replies (join/state
broadcasts, spectator-only frame relays). `UdpRelay` binds a non-blocking UDP
socket, drains datagrams through the server, and sends the replies, making the
protocol usable over the network (localhost relay is covered by tests).

### State streaming

`chunk_snapshot(frame, checksum, bytes)` splits a serialized snapshot into
1 KiB `Snapshot` chunks (at most 256 chunks / 256 KiB). The host relays them to
spectators only. `SnapshotAssembler` reassembles chunks, rejects stale frames,
duplicate indices, inconsistent checksums/totals, oversized chunks, and
payloads above 256 KiB, and returns the complete byte payload once every chunk
has arrived.

`snapshot_messages(sim, frame)` serializes the simulation with
`SimState::encode_snapshot` (version + asset id + state) and chunks it.
`SpectatorSnapshotReceiver::accept` reassembles the chunks, restores the state
with `SimState::decode_snapshot` (keeping the spectator's assets and rejecting
mismatched versions/assets), and verifies the restored checksum. A 256 KiB
snapshot cap applies on both ends.

`SpectatorSync` accepts increasing rollback frame numbers, rejects stale or
duplicate frames, counts gaps as `missed_frames`, and reports `lagging()` once
more than 60 frames (one second) are missing.

### Direct spectator mode

`SnapshotBroadcaster` binds a UDP socket and streams the current simulation
directly to spectators that send a `Join { spectator: true }` datagram. It
drains joins/leaves with `poll()` and sends a complete snapshot every
`interval_ticks` frames (6 by default) with `broadcast()`; dropped datagrams
are recovered by later re-broadcasts.

`SpectatorClient::connect(host)` joins that socket, polls datagrams into a
`SpectatorSnapshotReceiver`, applies complete snapshots with the spectator's
own assets, verifies the checksum, and retries the join periodically.
`status_text()` reports the current frame/checksum or the waiting state.

The app exposes this as `--broadcast <bind-addr>` on the host and
`--spectate <host-addr>` on the viewer. The viewer does not tick locally; it
renders the restored snapshots and draws the status line in the top-left
corner.

### LAN discovery

`DiscoveryResponder` answers UDP requests equal to `MANATO_DISCOVER_V1` with a
`MANATO_LOBBY_V1:`-prefixed, fixint-bincode `DiscoveryInfo` (name, relay
address, player/spectator counts, phase). Responses are capped at 4 KiB and
names at 64 characters. `DiscoveryBrowser` broadcasts the request and collects
up to 64 unique relay addresses. The default LAN port is UDP 27015.

### NAT address discovery

`stun_binding_request` builds an RFC 5389 request and `parse_stun_mapped`
extracts `XOR-MAPPED-ADDRESS` (or `MAPPED-ADDRESS`) for IPv4/IPv6.
`stun_public_address(server, timeout)` sends one request and returns the
mapped public address. This reports the NAT mapping; it does not itself punch
a hole or relay traffic.

`Matchmaker`/`best_lobby` rank discovered lobbies: Closed lobbies are excluded,
then Open > Starting > InMatch, then player count, spectator count, and name.

The app exposes:

- `--discover <addr>`: print LAN lobbies for one second,
- `--stun <addr>`: print the public UDP address,
- `--matchmake <addr>`: discover lobbies, list them, and start the spectator
  client against the best joinable relay (unless `--spectate` overrides it).

`LobbyClient` covers player slots: it joins a relay, tracks `Welcome`/`State`,
marks itself ready, and sends `Start` when it is the host. The app's
`--matchmake-play <discovery-addr>` discovers lobbies, prints them, joins the
best relay as a player, marks ready, and reports the resulting lobby state.

For match handoff, players call `announce_netplay(port)` to publish their GGRS
UDP port in `LobbyMember.netplay_port`. When the host sends `Start`, the relay
broadcasts `State` plus a `Peer { address }` message whose address uses the
opponent's relay IP and announced GGRS port. `LobbyClient::match_handoff()`
turns that into `MatchHandoff { local_player, remote }`, and
`Netplay::from_handoff(sim, handoff, bind)` opens the GGRS session.

Automatic NAT hole punching is not modeled, and the app's headless
`--matchmake-play` mode reports the lobby state rather than entering the match
loop itself.

## 4. Security notes

- Datagrams are decoded with a 4 KiB bincode limit and fixint encoding, so a
  hostile length prefix cannot force a large allocation.
- `LobbyServer` authorizes mutations by sender address: `Leave`/`Ready` must
  come from that member's peer, and `Start`/`Frame` must come from the host.
  Frames are relayed only to spectators.
- The protocol is unauthenticated plaintext UDP. Use it on trusted networks or
  wrap it in a secure transport; IP spoofing cannot be prevented at this layer.
- Joins are rate-limited per IP (5 joins per 10 seconds by default) and names
  are capped at 32 characters so snapshot messages fit the 4 KiB datagram
  limit. Hosts exposing a public relay should still add global bandwidth
  limits.

## 5. Not modeled

- Lobby discovery/announcement, NAT traversal, and matchmaking.
- Spectator UI/rendering integration and a continuously running host broadcast
  loop (the serialization, chunk transport, reassembly, restore, and checksum
  verification are implemented).
- Chat/moderation, spectator input isolation beyond the role flags.
