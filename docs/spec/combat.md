# Combat and collision specification

**Status: APPROVED / FROZEN.** Derived from the MIT-licensed Ikemen-GO
source (`src/collision.c`, `src/char.go`, `src/bytecode.go`), public M.U.G.E.N
HitDef documentation, and observed content.

## 1. Scope

Define hit detection and combat resolution for:

- Clsn1/Clsn2 boxes
- HitDef activation and parameters
- Guarding, guard flags, and guard damage
- Hit pause and super pause
- Hit/guard velocities and hit times
- HitOnce, multi-hit, juggle points
- Throws and target binding
- HitOverride and ReversalDef
- Projectile and helper hits
- Fall states, KO, and corner push

## 2. Collision boxes

- Clsn1 boxes are attack boxes.
- Clsn2 boxes are hurt boxes.
- Boxes are transformed from character-local space to world space using
  position, facing, and scale.
- Facing mirrors X; normalized rectangles are required.
- Empty attack or hurt boxes mean no collision.
- Projectiles use their current animation frame’s Clsn1 boxes transformed by
  projectile position and facing.
- Helpers use their current animation frame boxes like fighters.

## 3. HitDef activation

A `HitDef` controller becomes active when its triggers pass. It remains active
until:

- the state changes,
- another HitDef replaces it,
- the controller is reset, or
- the hit is consumed according to `hitonce`/`numhits` rules.

Each activation gets a unique `activation` id used for per-swing hit tracking.

## 4. HitDef parameters

Required/used parameters:

```
attr = S, NA
damage = 23, 0
guard.damage = 5
hitflag = MAF
guardflag = MA
animtype = Light|Medium|Heavy|Back|Up|DiagUp
ground.type = High|Low|Trip
air.type = High|Low|Trip
ground.velocity = x, y
air.velocity = x, y
airguard.velocity = x, y
ground.hittime = N
air.hittime = N
ground.slidetime = N
guard.hittime = N
guard.slidetime = N
pausetime = attacker, defender
sparkno = N
guard.sparkno = N
sparkxy = x, y
hitsound = group, index
guardsound = group, index
hitonce = 0|1
fall = 0|1
fall.recover = 0|1
fall.time = N
fall.damage = N
fall.xvelocity = x
fall.yvelocity = y
air.juggle = N
priority = N[, Hit|Miss|Dodge]
```

Unknown parameters are preserved and ignored with a warning.

### 4.1 Documented defaults and value codes

The public M.U.G.E.N 1.1 HitDef reference defines these defaults. They are
applied when a `HitDef` controller omits the parameter:

- `ground.type` is `High`. `air.type` inherits `ground.type` when omitted.
  Type names are case-insensitive; the single-letter forms `L` and `T` used
  by Ikemen-GO content are also accepted.
- `ground.velocity` and `air.velocity` are `0, 0`.
- `ground.hittime` is `0`, `ground.slidetime` is `0`, and `air.hittime` is
  `20`.
- `guard.hittime` inherits `ground.hittime` when omitted.
  `guard.slidetime` inherits `guard.hittime` when omitted.
- Guard hits use `guard.hittime`/`guard.slidetime`; unguarded ground hits use
  `ground.hittime`/`ground.slidetime`; air hits use `air.hittime`.
- Ikemen-GO preserves a reported M.U.G.E.N implementation quirk where
  M.U.G.E.N-version characters default `guard.hittime` to
  `ground.slidetime` instead. This engine follows the published M.U.G.E.N
  reference (`ground.hittime`) because the clean-room rules forbid verifying
  the binary quirk. This is an explicit compatibility decision, not an
  oversight.

`GetHitVar(type|groundtype|airtype)` reports the M.U.G.E.N/Ikemen-GO type
codes: `0` None, `1` High, `2` Low, `3` Trip. `GetHitVar(type)` returns the
air type when the defender was airborne at contact and the ground type
otherwise.

`ground.type = None`/`air.type = None` is a pause-only contact. It applies the
HitDef `pausetime` to both players (attacker self-pause and defender hit
shake), consumes the contact for the current activation, and performs no
damage, velocity, spark, sound, or state change. `air.type` still inherits
`ground.type`, so a ground `None` also makes air contact pause-only.

Corner-push defaults follow the documented legacy rule: when
`ground.cornerpush.veloff` is omitted, an air attack (`attr` state types
contain `A`) gets `0`, otherwise it gets `1.3 * guard.velocity.x`, where
`guard.velocity` falls back to `ground.velocity` before the multiplication.
`air.cornerpush.veloff` inherits the resulting ground value when omitted.
Explicit values always win. `guard.cornerpush.veloff` and
`airguard.cornerpush.veloff` are not separately modeled; guarded contacts use
the ground/air corner-push value selected by the defender's state.

## 5. Hit resolution order

For each attacker and defender pair:

1. Skip if either is inactive, or the defender is not a valid target.
2. Skip if the attack activation already hit the defender.
3. Check `NotHitBy`/`HitBy` filters.
4. Check `hitflag` against the defender’s state type.
5. Check attack/hurt box overlap.
6. Check `HitOverride`; if matched, redirect the defender and consume the hit.
7. Check `ReversalDef`; if matched, reverse the hit.
8. Determine guarding from input, state, and `guardflag`.
9. Apply hit or guard damage, velocities, timers, and state changes.
10. Spawn spark/sound and apply camera shake.
11. Record the defender as hit for this activation.

## 6. Guarding

A defender guards when:

- they are holding back or the `blocking` command is active,
- they are in a guard-capable state,
- the attack’s `guardflag` matches their state type (high/low/air),
- `AssertSpecial` does not disable that guard type,
- the attack is not unguardable.

Guard damage:

- use `guard.damage` if positive, else `damage / 4`,
- apply attack/defence multipliers,
- never reduce life below zero through guard alone unless the content says so.

## 7. Hit reaction

- Damage = base damage × attacker `attack_mul` × defender `defence_mul`.
- Choose ground/air velocity and hit time from the defender’s state.
- Guard velocity is half of the ground/air guard velocity.
- Set `GetHitVar` values: damage, velocities, hit time, slide time, guard flag,
  fall flag, fall time, fall damage, hit id.
- Enter guard states 150/152/154 or hit states 5000/5010/5020 when present.
- Without common states, apply velocity and leave the fighter in place.
- Hit pause freezes the affected fighter; super pause freezes all.
- `pausetime`'s first value is the attacker's self-pause and its second value
  is the defender's hit shake. `HitPauseTime` returns the attacker's remaining
  self-pause, counting down one per frozen tick; a defender frozen by hit shake
  reports 0 (oracle O-29).

## 8. HitOnce, multi-hit, and juggle

- `hitonce = 1`: an activation can hit only one target.
- `numhits`/`hits` allow multiple hits per activation; track per target.
- Each successful hit increments the attacker’s hit count.
- `air.juggle` consumes defender juggle points; insufficient points means miss
  unless `nojugglecheck` is asserted.
- `MoveHit`, `MoveGuarded`, `MoveContact`, `HitCount`, and `UniqHitCount`
  reflect the current activation.

## 9. Throws

- A throw HitDef uses `p1stateno` and `p2stateno`.
- On hit, the attacker enters `p1stateno`; the defender enters `p2stateno` and
  is bound with `TargetBind`.
- `TargetState`, `TargetLifeAdd`, `TargetVelSet/Add`, and `TargetFacing` operate
  on the bound target.
- `TargetDrop` clears the binding.
- Throw damage is applied by the throw state, not by the initial HitDef.

## 10. Projectile and helper hits

- Projectiles carry their own HitDef and hit target list.
- Projectile hits use the projectile position for spark placement.
- When a projectile exhausts its hits it plays `projhitanim`; when its
  `removetime`/bounds end it plays `projremanim`. `-1` removes it immediately.
  The projectile stays inactive (no further hits) until that animation ends,
  then is removed. Entering either state replaces the velocity with
  `remvelocity` and clears acceleration.
- Helpers use their owner’s attack multipliers and sound banks.
- Helpers are not valid defenders in the first pass.

## 11. Fall and KO

- Fall states `5030/5040/5050` use `air.gethit.groundlevel` and fall damage.
- KO sets life to zero and enters the round KO phase.
- The round phase controls input locks and the next round.
- KO/dizzy/guard-crush extensions are deferred unless in the matrix.

## 12. Corner push and pushboxes

- Pushboxes separate players after movement.
- `PlayerPush = 0` disables separation for that fighter.
- Stage bounds clamp players unless `ScreenBound` disables an axis.
- Corner push velocity offsets are applied on hit when content specifies them.

## 13. Error handling

- Missing HitDef fields use documented defaults.
- Invalid attributes never match.
- Missing common states fall back to velocity-only reactions.
- Do not panic on missing targets, helpers, or projectiles.
- Log once per invalid parameter rather than per tick.

## 14. Test plan

- Attack/hurt overlap and facing transforms.
- Hit vs guard, high/low/air guard flags.
- Guard damage and guard state selection.
- Hit pause, super pause, and hit shake.
- HitOnce and multi-hit.
- Juggle point exhaustion.
- Throw bind/state/life flow.
- HitOverride and ReversalDef.
- Projectile and helper hits.
- Corner push and pushbox separation.
- KO/round transition.
- Bundled roster and stock KFM/Ryu combat.

## 15. Sources

- Ikemen-GO `src/collision.c`, `src/char.go`, `src/bytecode.go` (MIT).
- Public M.U.G.E.N HitDef/collision documentation.
- Observed character combat data.
