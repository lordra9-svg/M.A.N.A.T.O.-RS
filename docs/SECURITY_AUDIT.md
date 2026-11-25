# Security audit

**Date:** 2026-11-25  
**Baseline:** `6c513fa` (documentation release v0.1.197)  
**Scope:** all workspace crates, bundled tools, GitHub workflows, content
loaders, and network protocols in this repository.

This is an internal, source-level audit. It is not a third-party penetration
test, a formal certification, or a guarantee that the engine is free of
defects. It records what was checked, what was fixed, and what remains.

## 1. Method

| Check | Tool / method |
| --- | --- |
| Known dependency advisories | `cargo audit` 0.23.0 against the RustSec database (1,568 advisories, 428 locked crates) |
| Build and lint | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` |
| Tests | `cargo test --workspace` with and without `MUGEN_ROOT` |
| Unsafe review | `grep` for `unsafe`, manual review of every occurrence |
| Parser review | Manual review of SFF v1/v2, PCX, PNG, SND, DEF, AIR, CNS/CMD, stage, screenpack |
| Network review | Manual review of GGRS wrapper, lobby, relay, discovery, NAT punch, STUN/TURN, TLS |
| Sandbox review | Lua storyboard/arcade, `SaveFile`/`LoadFile` paths |
| Secret scan | Repository-wide search for passwords, tokens, keys, and private-key blocks |
| Tooling review | Python oracle/generator scripts and `tools/build-release.sh` |
| Supply chain | Workflow permissions, action pinning, release artifact flow |

## 2. Dependency audit

`cargo audit` reported **0 vulnerabilities** and **5 unmaintained-crate
warnings**. There is no fixed upgrade for any warning.

| Crate | Version | Advisory | Exposure | Disposition |
| --- | --- | --- | --- | --- |
| `bincode` | 1.3.3 | RUSTSEC-2025-0141 (unmaintained) | Direct snapshots, lobby/discovery, GGRS (transitive) | Accepted; decoding is size-limited at every network and file boundary. Track a future bincode 2 or alternate codec migration. |
| `instant` | 0.1.13 | RUSTSEC-2024-0384 (unmaintained) | Transitive via GGRS only | Accepted; no engine API exposure. Revisit when GGRS updates. |
| `paste` | 1.0.15 | RUSTSEC-2024-0436 (unmaintained) | Transitive build-time macro | Accepted; does not execute at runtime. |
| `rustls-pemfile` | 2.2.0 | RUSTSEC-2025-0134 (unmaintained) | TURN TLS certificate/key loading | Accepted; simple PEM parsing with bounded use. Candidate for replacement by `rustls-pki-types` PEM helpers. |
| `ttf-parser` | 0.25.1 | RUSTSEC-2026-0192 (unmaintained) | Transitive via winit/ab_glyph | Accepted; no font files are parsed by this path for game content. |

No advisory required an emergency upgrade. The unmaintained status is a
maintenance risk, not a known exploit.

## 3. Findings and fixes

Severity is for the project's normal deployment: a local game client that may
load community content and may expose optional network services.

| ID | Severity | Area | Finding | Status |
| --- | --- | --- | --- | --- |
| SA-01 | Medium | SFF/PNG | A 16-bit PNG caused out-of-bounds writes in the 8-bit conversion path (`Grayscale`, `RGBA`, `GrayscaleAlpha`), and PNG-declared dimensions were not checked against the SFF limit. A malformed asset could panic or over-allocate. | **Fixed**: reject non-8-bit PNGs, validate dimensions, verify payload length before conversion. Test `v2_sixteen_bit_png_is_rejected_not_written_out_of_bounds`. |
| SA-02 | Medium | SFF v1/v2 | Sprite headers could declare many 8192×8192 sprites; each allocation is 256 MiB, allowing memory exhaustion from a small file. | **Fixed**: file-wide decoded-pixel budget (`32M` pixels ≈ 128 MiB RGBA). Test `v2_sprite_budget_rejects_max_dimension_bomb`. |
| SA-03 | Medium | Relay / GGRS | `RelayGgrsSocket` decoded relay payloads with unbounded `bincode::deserialize`. A malicious relay or peer could send a length prefix that forces a huge allocation. | **Fixed**: fixed-int decoder limited to the datagram payload size. |
| SA-04 | Low | Relay auth | `AuthRequest::decode` used unbounded bincode after a 512-byte frame check; a hostile length prefix could still request a large allocation. | **Fixed**: fixed-int decoder limited to the frame size. |
| SA-05 | Medium | STUN/TURN | `Message::verify_integrity` could panic while slicing a malformed MESSAGE-INTEGRITY attribute supplied by a remote peer. | **Fixed**: checked slice bounds and full-message length before verification. |
| SA-06 | Medium | TURN | The UDP TURN server accepted unlimited allocations and the TCP/TLS server unlimited connections; an authenticated or unauthenticated peer could exhaust sockets and memory. | **Fixed**: 256 live UDP allocations, 128 TCP/TLS connections, expired UDP allocations pruned, 508 response when full. |
| SA-07 | Medium | Arcade Lua | Arcade-path Lua ran without a memory limit, unlike the storyboard sandbox. A malicious path script could exhaust the process. | **Fixed**: 4 MiB memory limit and 1 MiB source cap. |
| SA-08 | Low | Lua scripts | Storyboard and arcade scripts were read fully before any size check. | **Fixed**: 1 MiB file/source cap before reading or executing. |
| SA-09 | Low | Audio | A channel count above 255 was truncated when filling PulseAudio's `u8` channel field. | **Fixed**: reject counts outside 1–255. |
| SA-10 | Medium | TURN auth | UDP and TCP TURN servers used the fixed nonce `manato-nonce`, allowing precomputed or cross-restart replay of authenticated requests. | **Fixed**: fresh 128-bit OS-random nonce per server instance. Same-process replay is still possible; see residual risks. |

## 4. Controls that passed review

- **Lobby and discovery decoding** uses fixed-int bincode with explicit limits
  (4 KiB lobby datagrams, 4 KiB discovery bodies) and caps names at 32
  characters.
- **Spectator snapshots** are chunked at 1 KiB, capped at 256 KiB, and reject
  duplicate, stale, oversized, inconsistent, or checksum-mismatched input.
- **Disk snapshots** reject files over 64 MiB, bound bincode decoding to the
  payload length, and require both a format version and an asset-id match.
- **`SaveFile`/`LoadFile`** reject absolute paths and `..` escapes and resolve
  relative paths inside the executing character's directory.
- **Lua sandboxes** expose only `table`, `string`, and `math`; `io`, `os`, and
  `require` are unavailable, and the storyboard/arcade runtimes have memory
  caps.
- **TLS** uses rustls defaults, which verify server certificates and hostnames;
  the server accepts a configured certificate/key and does not enable
  dangerous verifiers.
- **Authentication MACs** use the `hmac` crate's constant-time verification;
  relay join requests include a nonce replay cache (bounded at 4,096 entries).
- **No secrets** were found in the repository. The only passwords are test
  fixtures (`alice`/`secret`) inside unit tests.
- **Python tooling** uses argument lists, not `shell=True`, and does not
  interpolate untrusted values into shell strings.
- **`unsafe`** appears only in `manato-audio/src/pulse.rs` for dynamically
  loading PulseAudio symbols. The library handle outlives the writer thread,
  the stream handle is freed once, and the DLL/function signatures match
  `pa_simple`. No other crate contains `unsafe`.

## 5. Residual risks and accepted trade-offs

| Risk | Severity | Notes / recommendation |
| --- | --- | --- |
| TURN authenticated request replay within one server lifetime | Low–Medium | The nonce is now random per process, but it does not rotate and there is no seen-request cache. UDP retransmissions make naive duplicate rejection unsafe. Add nonce expiry/rotation or a short-lived transaction cache before exposing TURN to hostile networks. |
| Netplay is plaintext and unauthenticated | Informational / by design | GGRS traffic and relayed frames are not encrypted, and peers are not cryptographically authenticated. Relay join credentials authenticate registration only. Use a trusted network/VPN or add a secure transport for public play. |
| No anti-cheat | Informational / by design | A modified peer can send arbitrary inputs or snapshots. Determinism/checksums detect desyncs, not cheating. |
| Local content is trusted | Low | DEF/CNS/AIR/SND/stage text files are read without a generic size cap. Malformed or huge local files can still consume memory. Do not load untrusted content without an external sandbox/resource limit. |
| Symlink escape in the save/load sandbox | Low | The path check rejects `..`, but a symlink inside a character directory can still point outside it. Operators should not unpack untrusted content into a trusted character directory. |
| CI actions pinned to tags, not commit SHAs | Low | `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `actions/upload-artifact@v4`, `actions/download-artifact@v4`, and `softprops/action-gh-release@v2` are tag-pinned. For a hardened supply chain, pin full SHAs and use Dependabot/Renovate. |
| No fuzzing or Miri in CI | Low | Parsers and network decoders received manual review and targeted tests. Add `cargo-fuzz` targets for SFF/PCX/PNG/SND and the lobby/relay/STUN decoders, plus Miri for the Pulse unsafe layer. |
| Unmaintained dependencies | Low | See Section 2. Monitor GGRS/bincode and replace `rustls-pemfile` when convenient. |

## 6. Reproduction

```sh
# Advisory scan (requires: cargo install cargo-audit --locked)
cargo audit

# Build gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
MUGEN_ROOT=/path/to/mugen cargo test --workspace

# Unsafe inventory
grep -rn "unsafe" crates --include="*.rs"
```

The dependency scan was last run on 2026-11-25 and reported 428 locked crates,
0 vulnerabilities, and 5 unmaintained warnings. Re-run it whenever
`Cargo.lock` changes; advisory data changes independently of this repository.

## 7. Recommended next steps

1. Add a CI job for `cargo audit` and pin all workflow actions by SHA.
2. Add `cargo-fuzz` targets for `SffFile::from_bytes`, `decode_pcx`,
   `decode_png`, `SoundBank::from_bytes`, `LobbyMessage::decode`,
   `AuthRequest::decode`, and `Message::parse`/`verify_integrity`.
3. Add a TURN nonce rotation/expiry policy and a bounded replay cache that
   tolerates UDP retransmissions.
4. Replace `rustls-pemfile` with `rustls-pki-types` PEM helpers when the
   dependency tree is next updated.
5. If user-supplied content becomes a supported threat model, add a generic
   content-size limit and symlink-safe path resolution across all loaders.
