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
msg_status_effect = "0.1"
bevy = "0.16"
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
fn apply_speed_boost(mut commands: Commands, entity: Entity) {
    commands.trigger_targets(
        ApplyStatusEffect(SpeedModifier(ValueModifier::Percent(50.0))),
        entity
    );
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
    trigger: Trigger<ApplyStatusEffect<SpeedModifier>>,
    mut q_speed: Query<&mut Speed>,
) {
    let entity = trigger.target();
    if let Ok(mut speed) = q_speed.get_mut(entity) {
        // Custom logic
        trigger.event().0.apply(&mut speed, 1.0);
    }
}

fn plugin(app: &mut App) {
    status_effect_observer!(app, SpeedModifier, on_apply_speed);
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
    commands.trigger_targets(
        ApplyStatusEffect(HealthModifier(ValueModifier::Val(50.0))),
        player
    );

    // Increase speed by 30% (with diminishing returns)
    commands.trigger_targets(
        ApplyStatusEffect(SpeedModifier(ValueModifier::Percent(30.0))),
        player
    );
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
| 0.1                 | 0.16 |

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

Contributions are welcome! This crate is part of the [MolecularSadism](https://github.com/MolecularSadism) game development libraries.
