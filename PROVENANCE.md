# Provenance and licensing notes

M.A.N.A.T.O.-RS is an independent Rust implementation of the M.U.G.E.N runtime.
It is **not** a port of M.U.G.E.N and contains no M.U.G.E.N source code,
binaries, or game assets.

> Not affiliated with Elecbyte. M.U.G.E.N is a trademark of Elecbyte.

## Reference used

M.U.G.E.N itself is closed source. This project used three sources of
behavioral knowledge:

1. **Public format documentation and community knowledge** for file layouts,
   trigger/controller semantics, and the coordinate system.
2. **Stock and user-provided content** as input data. Content is read in place
   from the user's M.U.G.E.N installation and is never copied into this
   repository.
3. **The `mugen-reimpl` C reference** supplied with the project, consulted for
   ambiguous behavior (for example SFF v2 sprite prefixes, palette handling, and
   air-state landing rules). No C code was copied or transliterated into Rust;
   the Rust modules were written against the observed behavior and validated by
   running the same content.

Important provenance detail: the `mugen-reimpl` README states that its
architecture was **derived from REA/Ghidra analysis of M.U.G.E.N 1.1b1, 1.0,
and WinMUGEN Plus binaries**. The reference tree also contains no
`LICENSE`/`COPYING` file.

This project did not run Ghidra or REA, and did not decompile or disassemble any
M.U.G.E.N executable. However, because the same contributors read the
reverse-engineered C reference and wrote the Rust code, **this is not a strict
clean-room implementation**. It should be described as an independent,
behavior-compatible reimplementation with a documented reference.

## Provenance of specific decisions

- **SFF v1** uses embedded PCX images with inline palettes. The PCX RLE stream is
  decompressed into `bytes_per_line * height` and then tightened to `width`.
- **SFF v2** indexed sprites (RLE8, RLE5, LZ5) and PNG sprites begin with a
  four-byte little-endian uncompressed-size field. PNG8 sprites are indexed
  images whose own `PLTE` may be a dummy; the real colors come from the SFF
  palette node, so the decoder keeps raw indices and maps them itself.
- **Statedef defaults** follow the CNS documentation: `type = S`,
  `movetype = I`, `physics = N`; an explicit `U` means "unchanged". Entering
  state 0 therefore clears an attack/hit move type, which the CPU uses to decide
  when it may act.
- **Landing** from any ordinary `physics = A` state transitions to state 52 when
  it exists, matching the observed behavior of stock air attacks that do not
  carry their own `ChangeState` on landing.
- **Animation timing** treats `-1` as "display forever" and reports `AnimTime`
  as ticks remaining, with one tick at `total_time` before wrapping so
  `AnimTime = 0` triggers fire.

## Assets and licensing

- The engine code and tools are MIT licensed (see `LICENSE`).
- No M.U.G.E.N content is distributed with this repository. Characters, stages,
  sprites, sounds, and scripts are loaded from a user-supplied root.
- Stock content (for example `mugen-1.1b1/chars/kfm`) remains the property of
  its original authors and is used only as local test input.
- Original test characters and the Training Stage are referenced from the
  project's external content directory and are not vendored here.
- The custom characters' sound banks are original synthesized effects. They are
  produced by `tools/gen_sfx.py` and live in the external content directory, not
  in this repository.
- Third-party Rust dependencies retain their own licenses. Run
  `cargo deny check licenses` (or `cargo license`) before redistributing a
  binary.

## What was deliberately not done

- This project did not decompile or disassemble M.U.G.E.N executables.
- No `mugen-reimpl` C functions were copied into Rust.
- No copyrighted sprites, sounds, scripts, or stages were bundled.
- No attempt was made to replicate undocumented engine bugs; compatibility is
  driven by content behavior and public semantics.

## Publishing notes

- Keep the external content directory out of the repository.
- If you distribute the generated character sound banks separately, include
  `tools/gen_sfx.py` and state their license.
- Do not describe this project as strict clean-room; the reference lineage above
  makes that claim inaccurate.
