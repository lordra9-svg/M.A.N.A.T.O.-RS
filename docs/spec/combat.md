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
affectteam = B|E|F
damage = 23, 0
guard.damage = 5
hitflag = MAF
guardflag = MA
animtype = Light|Medium|Heavy|Back|Up|DiagUp
ground.type = High|Low|Trip
air.type = High|Low|Trip
ground.velocity = x, y
air.velocity = x, y
down.velocity = x, y
down.hittime = N
down.bounce = 0|1
down.cornerpush.veloff = F
yaccel = accel
airguard.velocity = x, y
ground.hittime = N
air.hittime = N
ground.slidetime = N
guard.hittime = N
guard.slidetime = N
guard.ctrltime = N
guard.dist = N
mindist = x, y
maxdist = x, y
snap = x, y
p1facing = -1|0|1
p1getp2facing = -1|0|1
p2facing = -1|0|1
airguard.ctrltime = N
pausetime = attacker, defender
guard.pausetime = attacker, defender
sparkno = N
guard.sparkno = N
sparkxy = x, y
hitsound = group, index
guardsound = group, index
hitonce = 0|1
kill = 0|1
guard.kill = 0|1
fall = 0|1
air.fall = 0|1
forcestand = 0|1
forcenofall = 0|1
fall.recover = 0|1
fall.time = N
fall.recovertime = N
fall.kill = 0|1
palfx.time = N
palfx.mul = r, g, b
palfx.add = r, g, b
envshake.time = N
envshake.freq = F
envshake.ampl = N
envshake.phase = F
fall.envshake.time = N
fall.envshake.freq = F
fall.envshake.ampl = N
fall.envshake.phase = F
fall.damage = N
fall.xvelocity = x
fall.yvelocity = y
air.juggle = N
priority = N[, Hit|Miss|Dodge]
id = N
chainid = N
nochainid = N[, N]
```

Unknown parameters are preserved and ignored with a warning.

### 4.1 Documented defaults and value codes

The public M.U.G.E.N 1.1 HitDef reference defines these defaults. They are
applied when a `HitDef` controller omits the parameter:

- `ground.type` is `High`. `air.type` inherits `ground.type` when omitted.
  Type names are case-insensitive; the single-letter forms `L` and `T` used
  by Ikemen-GO content are also accepted.
- `affectteam` defaults to `E` (enemy team). `B` allows both teams and `F`
  allows only the same team.
- `ground.velocity` and `air.velocity` are `0, 0`.
- `down.velocity` inherits `air.velocity` when omitted.
- `down.bounce` defaults to `0`. When `1` and `down.velocity.y` is non-zero,
  the next fall ground impact rebounds once using `fall.xvelocity` and
  `fall.yvelocity`; when `0`, those rebound velocities are cleared.
- `down.hittime` defaults to `20` (the engine’s oracle-compatible value; the
  public reference does not state a default). It applies only when
  `down.velocity.y` is zero.
- `yaccel` defaults to the documented localcoord-dependent value:
  `0.35 * localcoord.width / 320`, which is `.35` at 320x240, `.7` at
  640x480, and `1.4` at 1280x720. An explicit `yaccel` overrides it.
- `ground.hittime` is `0`, `ground.slidetime` is `0`, and `air.hittime` is
  `20`.
- `fall.recovertime` is `4`.
- `air.fall` inherits `fall` when omitted. It controls the fall flag for an
  airborne defender; `fall` controls it for a grounded defender.
- `forcestand` defaults to `1` when `ground.velocity.y` is non-zero, otherwise
  `0`. When active, a crouching grounded defender is moved to the standing hit
  reaction; airborne defenders ignore it.
- `forcenofall` defaults to `0`. When set and the hit itself is not a fall,
  it clears the defender’s existing fall flag. It has no effect when the
  selected fall parameter is set.
- `fall.kill` is `1`; when `0`, fall damage clamps life at `1` instead of
  KO'ing.
- `kill` and `guard.kill` are `1`; when `0`, a normal or guarded hit
  respectively clamps life at `1` instead of KO'ing.
- `palfx.time` is `0` (disabled), `palfx.mul` is `1,1,1`, and `palfx.add` is
  `0,0,0`.
- `envshake.time` is `0` (disabled), `envshake.freq` is `60`, and
  `envshake.ampl` is `-4`. `envshake.phase` defaults to the EnvShake rule:
  `90` when the frequency is at least `90`, otherwise `0`.
- `fall.envshake.time` is `0` (disabled), `fall.envshake.freq` is `60`, and
  `fall.envshake.ampl` is `-4`. `fall.envshake.phase` defaults to the EnvShake
  rule: `90` when the frequency is at least `90`, otherwise `0`.
- `guard.hittime` inherits `ground.hittime` when omitted.
  `guard.slidetime` inherits `guard.hittime` when omitted.
  `guard.ctrltime` inherits `guard.slidetime`, and `airguard.ctrltime`
  inherits `guard.ctrltime`.
- `guard.pausetime` inherits `pausetime` when omitted and supplies both the
  attacker self-pause and defender shake on guarded contacts.
- `guard.dist` defaults from the character’s `[Size] attack.dist`
  (`160` if absent) for melee HitDefs and `proj.attack.dist` (`90` if absent)
  for projectiles. An explicit `guard.dist`/`guard.dist.width` overrides it.
- `mindist` and `maxdist` default to no change (unset per axis). A missing Y
  component leaves that axis unconstrained.
- `snap` defaults to no change (unset per axis). A set axis overrides both
  `mindist` and `maxdist` on that axis.
- `p1facing`, `p1getp2facing`, and `p2facing` default to `0` (no change).
  `p1getp2facing` overrides `p1facing` when non-zero. Facing changes apply
  only to unguarded successful hits.
- `id` defaults to `0` and is reported as `GetHitVar(hitid)`. `chainid`
  defaults to `-1` and is recorded as `GetHitVar(chainid)`.
- `nochainid` defaults to `-1, -1` (no blocked chain IDs).
- `airguard.velocity` defaults to
  `(air.velocity.x * 1.5, air.velocity.y * 0.5)` when omitted. Guard
  velocities are applied directly; the engine does not halve them.
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

`GetHitVar(ctrltime)` follows the observable Ikemen-GO value: guarded ground
contacts report `guard.ctrltime`, guarded air contacts report
`airguard.ctrltime`, and unguarded contacts report the selected hit time
(`ground.hittime` or `air.hittime`). The public M.U.G.E.N trigger text
describes `ctrltime` only as the post-guard control time; the unguarded value
is kept because it is what content observes under the oracle runtime.

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
Explicit values always win. `guard.cornerpush.veloff` inherits
`ground.cornerpush.veloff`, and `airguard.cornerpush.veloff` inherits
`guard.cornerpush.veloff`; guarded ground and air contacts select those
values instead of the unguarded ones. `down.cornerpush.veloff` inherits
`ground.cornerpush.veloff` and is selected for grounded downed hits.

## 5. Hit resolution order

For each attacker and defender pair:

1. Skip if either is inactive, the defender is not a valid target, or
   `affectteam` excludes the defender’s team.
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
- the defender is within the attack’s `guard.dist` X distance,
- they are in a guard-capable state,
- the attack’s `guardflag` matches their state type (high/low/air),
- `AssertSpecial` does not disable that guard type,
- the attack is not unguardable.

Guard damage:

- use `guard.damage` if positive, else `damage / 4`,
- apply attack/defence multipliers,
- never reduce life below zero through guard alone unless the content says so.

`InGuardDist` is true when the opponent has an active HitDef (or is attacking)
within that HitDef’s `guard.dist`, or when one of the opponent’s active
projectiles is within its own projectile `guard.dist`. The comparison is
strict: distance must be less than `guard.dist`.

## 7. Hit reaction

- Damage = base damage × attacker `attack_mul` × defender `defence_mul`.
- A hit may KO only when the applicable `kill` flag is set: `kill` for a
  normal hit and `guard.kill` for a guarded hit. When disabled, life clamps
  at `1`.
- Choose ground/air velocity and hit time from the defender’s state.
- The defender’s `GetHitVar(fall)` flag comes from `air.fall` for an
  airborne hit and from `fall` for a grounded hit. Common fall states use it
  to choose the falling/knockdown reaction.
- `forcestand` changes a crouching grounded defender to the standing state
  type before the hit reaction state is selected, so state `5000` is used
  instead of `5010`.
- A non-fall hit preserves the defender’s existing fall flag;
  `forcenofall` clears it. A hit that itself sets `fall`/`air.fall` ignores
  `forcenofall`. Ending hitstun clears the persistent fall flag.
- Unguarded air hit states apply the HitDef `yaccel` as their vertical
  acceleration and expose it through `GetHitVar(yaccel)`. Ordinary air
  movement and air guard continue to use the character’s `movement.yaccel`.
- Guard velocity is applied directly: `guard.velocity` (or its
  `ground.velocity` fallback) for a grounded guard, and
  `airguard.velocity` for an air guard.
- Hitting a grounded lying-down defender requires the `D` hitflag. It uses
  `down.velocity`, enters state `5080`, and otherwise follows the normal
  damage/hitpause rules. A grounded downed hit uses `down.hittime` for
  `hittime`/`ctrltime`; a launched hit ignores it.
- A launched downed hit with `down.bounce = 1` rebounds once on its next fall
  ground impact using the stored fall velocities, then the pending bounce is
  cleared. `down.bounce = 0` disables the rebound.
- `mindist`/`maxdist` clamp the defender’s post-hit position relative to the
  attack origin and attacker facing. X bounds apply to every successful
  contact; Y bounds apply to unguarded hits and airborne defenders.
- `snap` sets the exact post-hit position on each provided axis, overriding
  the corresponding min/max bound. Its X axis is facing-relative; Y follows
  the same application conditions as the Y bounds.
- On unguarded hits, `p1getp2facing` sets P1 to P2’s facing (`1`) or its
  opposite (`-1`) and takes precedence over `p1facing`; `p2facing` sets the
  defender’s facing. Guarded contacts leave facing unchanged.
- Set `GetHitVar` values: damage, velocities, hit time, slide time, guard flag,
  fall flag, fall time, fall damage, hit id.
- `GetHitVar(hitshaketime)` returns the defender’s current hit-shake timer,
  including on guarded contacts. It counts down once per game tick and stops
  at zero; `HitShakeOver` becomes true at that point.
- `GetHitVar(hitid)` reports the connecting HitDef/projectile `id`, and
  `GetHitVar(chainid)` records its `chainid`. A HitDef with `chainid >= 0`
  can only hit a defender whose last recorded hit id matches. During hitstun,
  `nochainid` entries reject hits whose previous hit id matches them; `-1`
  entries are ignored.
- `GetHitVar(fallcount)` counts entries into the trip-shake state `5070` and
  the downed state `5100` during the current combo. The count persists while
  the defender remains in hitstun and resets when hitstun ends.
- `GetHitVar(fall.recovertime)` returns the HitDef’s
  `fall.recovertime` value (`4` by default).
- `GetHitVar(recovertime)` returns the remaining liedown recovery time. An
  unguarded hit seeds it from the defender’s `data.liedown.time` constant
  (`60` by default); it counts down once per tick while the defender is in
  state `5110`. Guarded contacts leave an existing value unchanged.
  Button-accelerated fast recovery is not modeled yet.
- `GetHitVar(fall.kill)` reports whether fall damage may KO (`1`/`0`).
- A HitDef `palfx.time` above zero replaces the defender’s palette effect
  with `palfx.add`/`palfx.mul` for that many ticks on any successful contact,
  guarded or unguarded. `palfx.time = 0` leaves an existing effect untouched.
- A HitDef `envshake.time` above zero starts the screen shake on any
  successful contact, guarded or unguarded, using its explicit
  freq/ampl/phase. It overrides the generic heavy-hit camera kick.
- `GetHitVar(fall.envshake.time|freq|ampl|phase)` reports the stored
  fall-impact shake. On landing, the engine applies that shake, and a
  `FallEnvShake` controller consumes the same values and clears the time so it
  cannot retrigger.
- Enter guard states 150/152/154 or hit states 5000/5010/5020 when present.
- Without common states, apply velocity and leave the fighter in place.
- Hit pause freezes the affected fighter; super pause freezes all.
- `pausetime`'s first value is the attacker's self-pause and its second value
  is the defender's hit shake. `HitPauseTime` returns the attacker's remaining
  self-pause, counting down one per frozen tick; a defender frozen by hit shake
  reports 0 (oracle O-29).
- Guarded contacts use `guard.pausetime` (falling back to `pausetime`) for
  both the attacker self-pause and defender shake.

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
