# Helper entity specification

**Status: DRAFT — awaiting approval.** Derived from the MIT-licensed Ikemen-GO
source (`src/char.go`, `src/bytecode.go`), public M.U.G.E.N documentation, and
observed content.

## 1. Scope

Add a general entity layer so characters can spawn helpers and use:

- `Helper`
- `BindToRoot`, `BindToTarget`, `BindToParent`
- `ParentVarSet`, `ParentVarAdd`
- `RootVarSet`, `RootVarAdd`
- `helper`, `parent`, and `root` redirects/triggers
- Helper hitdefs and projectiles

Out of scope for the first pass: helper-specific palette ownership, custom
helper shaders, and team/tag helpers.

## 2. Entity model

The simulation currently assumes exactly two fighters. It becomes:

```
players: [Fighter; 2]   // fixed player slots
helpers: Vec<Fighter>   // appended in spawn order
```

`Fighter` gains:

```
owner: Option<usize>    // helper owner / root player
parent: Option<usize>   // parent entity (usually the owner)
root: Option<usize>     // root player
helper_id: i32
helper_remove_time: i32
helper_destroy: bool
```

Entity addressing:

- `0`, `1`: players
- `2 + n`: helper `n`

`SimState::entity(who)` and `entity_mut(who)` resolve both players and helpers.
`SimState::opponent_of(who)` returns the opposing player for players and helpers.

## 3. Helper spawn

`Helper` controller parameters:

```
stateno = N
id = N
pos = x, y
postype = p1|p2|front|back|left|right
facing = 1|-1
velocity = x, y
removetime = N
keyctrl = 0|1
helpertype = normal|player
name = "..."
size.xscale / size.yscale
```

Spawn rules:

- The helper uses the owner’s `char_id` and sprite/animation/sound banks.
- Position is relative to `postype`; `pos.x` is facing-relative.
- Facing defaults to the owner’s facing.
- `stateno` is entered immediately.
- `removetime >= 0` removes the helper after that many ticks.
- `DestroySelf` sets `helper_destroy`; the helper is removed at the end of the
  tick.
- Helpers are appended in spawn order for deterministic checksums.

## 4. Update and state machine

Each tick:

1. Players run their normal update.
2. Helpers run the same state/controller pipeline:
   - current state controllers
   - physics/position integration
   - animation advance
   - helper removal timer
3. Helper special states `-3/-2/-1` run if present.
4. Helpers do not participate in round win/loss or camera focus.

Helpers use the owner’s `Character` data and their own variables, hit state,
animation clock, and physics.

## 5. Binding and redirects

- `BindToRoot`: position relative to the root player.
- `BindToParent`: position relative to the parent entity.
- `BindToTarget`: position relative to the owner’s current target.
- `ParentVarSet/Add`: write to the parent’s `vars`.
- `RootVarSet/Add`: write to the root player’s `vars`.
- `helper(id)` redirect: first helper with matching `helper_id`.
- `parent` redirect: helper’s parent; players redirect to self.
- `root` redirect: root player.
- Missing redirect targets make the trigger false and controller a no-op.

## 6. Collision

- Helper `HitDef`s can hit players.
- Helper `Projectile`s can hit players.
- Players do not target helpers by default in the first pass.
- Helper pushboxes do not separate players.
- Helper hit sparks and sounds use the owner’s banks.

## 7. Rendering

- Helpers are drawn using the owner’s character sprite bank.
- Draw order: stage, players, helpers, projectiles, front effects, HUD.
- Helpers respect facing, draw offsets, palette/transparency state, and
  afterimages.
- Helper sprites are included in the existing atlas banks; no new atlas is
  required.

## 8. Determinism

- Helpers are part of `SimState` and are cloned for rollback.
- Checksums include helper count, owner/parent/root, id, position, velocity,
  state, animation, timers, life, variables, hit state, and hitdef activation.
- Helper spawn order is deterministic.

## 9. Error handling

- Missing `stateno` or invalid animation: log and skip the helper.
- Helper count is capped (for example 32) to avoid runaway content.
- Missing redirect targets are false/no-op, not panics.
- Removal during iteration is deferred to the end of the tick.

## 10. Test plan

- Spawn a helper, verify state/animation/position.
- Helper runs state controllers and expires by `removetime`.
- `DestroySelf` removes a helper.
- `BindToRoot`/`BindToParent` update position.
- `ParentVarSet`/`RootVarSet` mutate the correct variables.
- `helper(0), stateno` and `root, life` redirects.
- Helper HitDef damages a player.
- Helper projectile damages a player.
- Rollback clone determinism with active helpers.
- Bundled roster and stock KFM/Ryu helper content.

## 11. Sources

- Ikemen-GO `src/char.go`, `src/bytecode.go`, `src/compiler_functions.go` (MIT).
- Public M.U.G.E.N Helper/Bind/ParentVar/RootVar documentation.
- Observed helper-using characters.
