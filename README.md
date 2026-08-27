# msg_status_effect

A generic status effect system for Bevy games with configurable scaling.

This crate provides a type-safe, observer-driven architecture for applying effects to game entities with support for diminishing/increasing returns.

## Features

- **Type-Safe**: Effect types are statically linked to their target components
- **Configurable Scaling**: Per-component power scaling for game balance (diminishing/increasing returns)
- **Observer-Based**: Uses Bevy's observer system for efficient event dispatch
- **Auto-Insert**: Missing components are automatically inserted with defaults
- **Organized Observers**: `status_effect_observer!` macro for organized entity hierarchy

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
msg_status_effect = { git = "https://github.com/MolecularSadism/msg_status_effect", tag = "v0.4.0" }
bevy = "0.18"
```

## Quick Start

```rust
use bevy::prelude::*;
use msg_status_effect::prelude::*;

// Define a component that will receive effects (must implement Default)
#[derive(Component, Default)]
pub struct Speed(pub f32);

// Define an effect type
#[derive(Event, Clone, Copy)]
pub struct SpeedModifier(pub ValueModifier);

// Implement the applicator trait
impl StatusEffectApplicator<Speed> for SpeedModifier {
    fn modifier(&self) -> ValueModifier { self.0 }
    fn apply(&self, component: &mut Speed, power: f32) {
        component.0 = self.0.apply_scaled(component.0, power);
    }
}

// Register in your plugin
fn plugin(app: &mut App) {
    app.add_plugins(StatusEffectPlugin::<Speed, SpeedModifier>::new(
        StatusEffectApplication::sqrt() // Diminishing returns
    ));
}

// Apply effects using triggers
fn apply_speed_boost(mut commands: Commands, query: Query<Entity, With<Speed>>) {
    if let Ok(entity) = query.single() {
        commands.trigger(ApplyStatusEffect {
            effect: SpeedModifier(ValueModifier::Percent(50.0)),
            entity,
        });
    }
}
```

## Value Modifiers

The `ValueModifier` enum supports two types of modifications:

```rust
// Flat additive value
ValueModifier::Val(10.0)     // +10 to current value

// Percentage modifier (in percentage points)
ValueModifier::Percent(50.0)  // +50% = 1.5x multiplier
ValueModifier::Percent(-10.0) // -10% = 0.9x multiplier
```

## Power Scaling

Power scaling controls how effects combine, enabling diminishing or increasing returns:

| Scaling | Power | Effect |
|---------|-------|--------|
| `LINEAR` | 1.0 | Standard addition/multiplication |
| `SQRT` | 0.5 | Diminishing returns |
| `CUBE_ROOT` | 1/3 | Strong diminishing returns |
| `SQUARE` | 2.0 | Increasing returns |
| `CUBE` | 3.0 | Strong increasing returns |

### Diminishing Returns Example

With `SQRT` scaling (power = 0.5):

```rust
// Flat value: (current^2 + val^2)^0.5
// 40 speed + 30 buff = sqrt(40^2 + 30^2) = 50 (not 70!)

// Percentage: current * multiplier^0.5
// 100 speed * 1.5^0.5 = ~122 (not 150!)
```

### Configuration

```rust
// Linear (default) - no diminishing returns
StatusEffectApplication::<Speed>::linear()

// Square root - moderate diminishing returns
StatusEffectApplication::<Speed>::sqrt()

// Cube root - strong diminishing returns
StatusEffectApplication::<Speed>::cube_root()

// Custom power
StatusEffectApplication::<Speed>::with_power(0.7)
```

## Plugin Setup

```rust
use msg_status_effect::prelude::*;

fn plugin(app: &mut App) {
    // Linear scaling (default)
    app.add_plugins(StatusEffectPlugin::<Speed, SpeedModifier>::default());

    // With diminishing returns
    app.add_plugins(StatusEffectPlugin::<Health, HealthModifier>::new(
        StatusEffectApplication::sqrt()
    ));

    // Custom power
    app.add_plugins(StatusEffectPlugin::<Armor, ArmorModifier>::new(
        StatusEffectApplication::with_power(0.7)
    ));
}
```

## Observer Macro

For custom effect handling, use the `status_effect_observer!` macro:

```rust
fn on_apply_speed(
    on: On<ApplyStatusEffect<SpeedModifier>>,
    mut q_speed: Query<&mut Speed>,
) {
    let entity = on.entity;
    if let Ok(mut speed) = q_speed.get_mut(entity) {
        // Custom logic
        on.effect.0.apply(&mut speed, 1.0);
    }
}

fn plugin(app: &mut App) {
    status_effect_observer!(app, SpeedModifier, on_apply_speed);
}
```

## Timed Status Holds

The `timed` module (re-exported from the prelude) generalizes *presence-as-state* status holds:
components an entity carries **only while actually held** — a stun, a snare, a root — so hot
queries exclude held entities with `Without<C>` instead of fetching and branching. It layers on
top of the same [`StatusEffectApplicator`]/[`ValueModifier`] abstractions as the rest of the crate.

What the module owns, and what stays in your game:

| Concern | Owned by the module | Stays in your game |
|---------|---------------------|--------------------|
| When a hold lifts | `ReleaseCondition<M>`: `Permanent` / `Time(Duration)` / `Milestone(M)` | the milestone type `M` (an `Ord` enum) and whatever reports it |
| Hold data | `StatusHold<M, S>`: release condition, optional restore payload `S`, runtime timer | what the restore payload *means* |
| Stacking | `ReleaseCondition::overwritten_by` + `StatusHold::next` | — |
| Ticking timed holds | `TimedStatus` + `tick_timed_status` + `TimedStatusPlugin<C>` | the concrete hold component |
| Releasing | `release_hold::<C>()` drops the component and fires `StatusReleased<C>` | observers deciding what release *does* |
| Duration scaling | `TimerStatusEffect` + `DurationModifier<C>` + `DurationModifierPlugin<C>` | which perk/item triggers it |

### Release conditions and milestones

`ReleaseCondition<M>` is generic over a host-defined milestone type `M` — an `Ord` enum whose
ordering encodes *strictness*. A stricter milestone (greater) satisfies a hold waiting on a laxer
one, so `StatusHold::released_by(reported)` is true when `reported >= required`:

```rust
use msg_status_effect::prelude::*;
use std::time::Duration;

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AnimationMilestone { Changed, CycleFinished, Finished }

// Waiting on CycleFinished: a Finished report (stricter) releases it, a Changed one does not.
let hold: StatusHold<AnimationMilestone, ()> = StatusHold {
    release: ReleaseCondition::Milestone(AnimationMilestone::CycleFinished),
    ..Default::default()
};
assert!(hold.released_by(&AnimationMilestone::Finished));
assert!(!hold.released_by(&AnimationMilestone::Changed));

// Hosts with no milestones use the default `()`.
let timed: StatusHold = StatusHold {
    release: ReleaseCondition::Time(Duration::from_secs(2)),
    ..Default::default()
};
assert!(!timed.released_by(&()));
```

### Stacking

`StatusHold::next(existing, incoming, current_state)` decides whether a re-application lands.
`overwritten_by` ranks conditions: `Permanent` always wins; a strictly longer `Time` wins (an
equal duration never refreshes); timers beat milestones both directions; a stricter milestone
wins. A fresh activation (`existing == None`, i.e. the component is absent) always lands and
captures `current_state`; a stack-up keeps the original restore payload rather than re-reading the
now-held live state.

### Ticking and the release seam

Add one `TimedStatusPlugin<C>` per hold component to tick its timer in `FixedUpdate` (or call
`tick_timed_status::<C>` yourself to place it in a custom set). When a timer finishes — or when any
host system decides a hold should lift — `release_hold::<C>()` removes the component and fires the
`StatusReleased<C> { entity, previous_state }` entity event. What release *does* is entirely up to
your observers: restore an FSM state, replay an animation, nothing at all. Racing release paths in
one frame announce a single release.

```rust
use bevy::prelude::*;
use msg_status_effect::prelude::*;
use std::time::Duration;

#[derive(Component)]
struct Rooted(StatusHold);

impl TimedStatus for Rooted {
    type Milestone = ();
    type Restore = ();
    fn hold(&self) -> &StatusHold { &self.0 }
    fn hold_mut(&mut self) -> &mut StatusHold { &mut self.0 }
}

fn plugin(app: &mut App) {
    app.add_plugins(TimedStatusPlugin::<Rooted>::default());
    app.add_observer(|on: On<StatusReleased<Rooted>>| {
        // React to the root lifting.
        let _ = on.entity;
    });
}
```

### Duration scaling

A hold with a runtime timer implements `TimerStatusEffect` to earn a `DurationModifier<C>` for
free, registered through `DurationModifierPlugin<C>`. It scales the hold's *remaining* time through
the same `ApplyStatusEffect`/power-scaling machinery — a resistance perk that shortens roots, say —
and, unlike `StatusEffectPlugin`, never inserts a component: scaling an entity that is not held is
a no-op instead of a spurious permanent hold.

```rust
use bevy::prelude::*;
use msg_status_effect::prelude::*;
use std::time::Duration;

#[derive(Component)]
struct Rooted(StatusHold);

impl TimerStatusEffect for Rooted {
    fn timer_mut(&mut self) -> Option<&mut Timer> { self.0.timer.as_mut() }
    fn set_duration(&mut self, duration: Duration) { self.0.set_duration(duration); }
}

fn plugin(app: &mut App) {
    app.add_plugins(DurationModifierPlugin::<Rooted>::default());
}

// Shorten a running root by 20%:
fn apply_resistance(mut commands: Commands, entity: Entity) {
    commands.trigger(ApplyStatusEffect {
        entity,
        effect: DurationModifier::<Rooted>::new(ValueModifier::Percent(-20.0)),
    });
}
```

## Complete Example

```rust
use bevy::prelude::*;
use msg_status_effect::prelude::*;

#[derive(Component, Default)]
struct Health { current: f32, max: f32 }

#[derive(Component, Default)]
struct Speed(f32);

#[derive(Event, Clone, Copy)]
struct HealthModifier(ValueModifier);

#[derive(Event, Clone, Copy)]
struct SpeedModifier(ValueModifier);

impl StatusEffectApplicator<Health> for HealthModifier {
    fn modifier(&self) -> ValueModifier { self.0 }
    fn apply(&self, component: &mut Health, power: f32) {
        let ratio = component.current / component.max;
        component.max = self.0.apply_scaled(component.max, power);
        component.current = component.max * ratio;
    }
}

impl StatusEffectApplicator<Speed> for SpeedModifier {
    fn modifier(&self) -> ValueModifier { self.0 }
    fn apply(&self, component: &mut Speed, power: f32) {
        component.0 = self.0.apply_scaled(component.0, power);
    }
}

fn plugin(app: &mut App) {
    // Health with linear scaling
    app.add_plugins(StatusEffectPlugin::<Health, HealthModifier>::default());

    // Speed with diminishing returns
    app.add_plugins(StatusEffectPlugin::<Speed, SpeedModifier>::new(
        StatusEffectApplication::sqrt()
    ));
}

fn apply_buffs(mut commands: Commands, player: Entity) {
    // Increase max health by 50
    commands.trigger(ApplyStatusEffect {
        effect: HealthModifier(ValueModifier::Val(50.0)),
        entity: player,
    });

    // Increase speed by 30% (with diminishing returns)
    commands.trigger(ApplyStatusEffect {
        effect: SpeedModifier(ValueModifier::Percent(30.0)),
        entity: player,
    });
}
```

## API Reference

### `ValueModifier`

```rust
impl ValueModifier {
    fn flat(value: f32) -> Self;        // Create flat modifier
    fn percent(percent: f32) -> Self;   // Create percent modifier
    fn apply(&self, current: f32) -> f32;                    // Apply linear
    fn apply_scaled(&self, current: f32, power: f32) -> f32; // Apply with scaling
    fn flat_value(&self) -> f32;        // Get flat value (or 0)
    fn percent_value(&self) -> f32;     // Get percent value (or 0)
    fn is_flat(&self) -> bool;
    fn is_percent(&self) -> bool;
    fn scaled_by(&self, factor: f32) -> Self;
}
```

### `StatusEffectApplicator<C>`

```rust
pub trait StatusEffectApplicator<C: MutableComponent>: Event + Clone {
    fn modifier(&self) -> ValueModifier;
    fn apply(&self, component: &mut C, power: f32);
}
```

### `StatusEffectPlugin<C, E>`

```rust
impl<C, E> StatusEffectPlugin<C, E> {
    fn new(config: StatusEffectApplication<C>) -> Self;
    fn default() -> Self; // Linear scaling
}
```

## Bevy Version Compatibility

| `msg_status_effect` | Bevy |
|---------------------|------|
| 0.4                 | 0.18 |
| 0.3                 | 0.18 |
| 0.2                 | 0.17 |
| 0.1                 | 0.16 |

## Migration Guide

### 0.3 → 0.4 (additive)

**No breaking API changes.** 0.4 adds the [`timed`](#timed-status-holds) module for
presence-as-state status *holds* (stuns, snares, roots) that lift on a release condition, with
stacking rules, an observable release seam, and reusable duration scaling. Everything from 0.3
is unchanged; a project that does not use `timed` needs no edits.

### 0.2 → 0.3 (Bevy 0.17 → 0.18)

**No breaking API changes for users of this crate.** The internals now use `commands.get_spawned_entity()` instead of `commands.get_entity()` for the auto-insert behavior, which aligns with Bevy 0.18's stricter entity state checking. This means attempting to apply an effect to an entity that has been reserved via `commands.spawn()` but not yet flushed will silently do nothing — the same as targeting a non-existent entity.

**Observer parameter type** remains `On<T>` (unchanged from Bevy 0.17).

### 0.1 → 0.2 (Bevy 0.16 → 0.17)

- Observer parameter changed from `Trigger<T>` to `On<T>`
- `trigger.target()` → `on.entity`
- `trigger.event()` → direct field access via `on.field_name` (through `Deref`)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

Contributions are welcome! This crate is part of the [MolecularSadism](https://github.com/MolecularSadism) game development libraries.
