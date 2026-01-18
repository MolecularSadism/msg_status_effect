# Timed Buff/Debuff System Plan

## Executive Summary

Extend `msg_status_effect` to support timed buffs and debuffs that automatically expire. The system uses a unified API where duration is optional (None = permanent), with configurable timeout behavior for effect reversal.

---

## Current State Analysis

### Strengths
- **Type-safe**: Effect types are statically linked to target components via `StatusEffectApplicator`
- **Configurable scaling**: Power-based diminishing/increasing returns
- **Observer-based**: Uses Bevy's modern observer pattern
- **ValueModifier**: Clean flat vs percentage modification abstraction

### Limitations
1. **No Duration**: Effects apply immediately and permanently
2. **No Effect Tracking**: Effects are anonymous; no way to list active effects
3. **No Revertibility**: Direct component mutation with no undo capability
4. **No Timeout Behavior**: No handling for what happens when effects expire

---

## Design Decision: Unified API with Optional Duration

### Core Concept

Effects use the same API but with optional duration. The `ActiveEffects<E>` component holds a `Vec` of effect instances, each with its modifier and optional duration. When duration expires, the effect is reversed based on `TimeOutBehavior`.

```rust
// Permanent effect (no duration)
ApplyStatusEffect(effect::WalkSpeed(ValueModifier::Percent(20.0)))

// Timed effect (with duration)
ApplyStatusEffect(effect::WalkSpeed(ValueModifier::Percent(20.0)))
    .with_duration(Duration::from_secs(30))
```

### Why This Approach

1. **API Consistency**: Same trigger pattern for permanent and timed effects
2. **Flexible Stacking**: Vec-based storage allows unlimited stacking
3. **Clear Timeout Semantics**: Explicit behavior for effect reversal
4. **Backwards Compatible**: Existing code continues to work (permanent by default)

---

## Architecture

### 1. Effect Instance Storage

```rust
/// A single instance of an applied effect.
#[derive(Clone, Debug, Reflect)]
pub struct EffectInstance {
    /// The modifier being applied
    pub modifier: ValueModifier,

    /// Time remaining (None = permanent)
    pub duration: Option<Timer>,

    /// For Additive timeout: the actual value that was added/multiplied
    /// Stored at application time for precise reversal
    pub applied_delta: Option<f32>,

    /// Optional source entity (e.g., the buff shrine that granted this)
    pub source: Option<Entity>,
}

/// Tracks all active instances of effect type E on this entity.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ActiveEffects<E: Event + Clone> {
    /// All active effect instances (can stack)
    pub instances: Vec<EffectInstance>,

    /// How to handle effect timeout/removal
    pub timeout_behavior: TimeOutBehavior,

    _marker: PhantomData<E>,
}
```

### 2. TimeOutBehavior

```rust
/// Defines how effects are reversed when they expire.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Reflect)]
pub enum TimeOutBehavior {
    /// Apply the inverse operation on timeout.
    ///
    /// Example: +10% buff → -10% on timeout
    ///
    /// This means if the base value changed while the buff was active,
    /// the removal affects the NEW value. This can be "abused" for
    /// interesting gameplay (intentional design choice).
    ///
    /// Math: On apply: value = modifier.apply(value)
    ///       On expire: value = modifier.inverse().apply(value)
    #[default]
    Multiplicative,

    /// Store the actual numeric change at application time.
    /// Subtract exactly that amount on timeout.
    ///
    /// Example: +10% buff on 150 → stores +15 → subtracts 15 on timeout
    ///
    /// This is "fair" - you always lose exactly what you gained,
    /// regardless of value changes in between.
    ///
    /// Math: On apply: delta = new_value - old_value; store delta
    ///       On expire: value = value - stored_delta
    Additive,
}
```

### 3. Extended Event API

```rust
/// Request to apply a status effect (permanent or timed).
#[derive(Event, Clone)]
pub struct ApplyStatusEffect<E: Event + Clone> {
    pub effect: E,
    /// None = permanent, Some = expires after duration
    pub duration: Option<Duration>,
    /// Optional source entity
    pub source: Option<Entity>,
}

impl<E: Event + Clone> ApplyStatusEffect<E> {
    /// Create a permanent effect (no expiration)
    pub fn permanent(effect: E) -> Self {
        Self { effect, duration: None, source: None }
    }

    /// Create a timed effect
    pub fn timed(effect: E, duration: Duration) -> Self {
        Self { effect, duration: Some(duration), source: None }
    }

    /// Add duration to make this a timed effect
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Set the source entity
    pub fn from_source(mut self, source: Entity) -> Self {
        self.source = Some(source);
        self
    }
}

// Convenience: wrap just the effect for permanent (backwards compatible)
impl<E: Event + Clone> From<E> for ApplyStatusEffect<E> {
    fn from(effect: E) -> Self {
        Self::permanent(effect)
    }
}
```

### 4. Fired Events

```rust
/// Fired when a timed effect expires naturally.
#[derive(Event)]
pub struct EffectExpired<E: Event + Clone> {
    pub effect: E,
    pub source: Option<Entity>,
}

/// Request to remove all instances of an effect type.
#[derive(Event)]
pub struct RemoveAllEffects<E: Event + Clone> {
    _marker: PhantomData<E>,
}

/// Request to remove effects from a specific source.
#[derive(Event)]
pub struct RemoveEffectsFromSource<E: Event + Clone> {
    pub source: Entity,
    _marker: PhantomData<E>,
}
```

### 5. Effect Categories (for UI)

```rust
/// Categorizes effects for UI display and game mechanics.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Reflect)]
pub enum EffectCategory {
    /// Beneficial effect (green, above health bar)
    Buff,

    /// Harmful effect (red, above health bar)
    Debuff,

    /// Neither beneficial nor harmful
    #[default]
    Neutral,
}

/// Trait for effects that have category information.
pub trait CategorizedEffect {
    fn category(&self) -> EffectCategory { EffectCategory::Neutral }
}
```

### 6. Plugin Configuration

```rust
/// Plugin for status effects on a specific component.
pub struct StatusEffectPlugin<C, E>
where
    C: MutableComponent + Default,
    E: Event + Clone + StatusEffectApplicator<C>,
{
    /// Power scaling for effect application
    pub config: StatusEffectApplication<C>,

    /// Default timeout behavior for this effect type
    pub timeout_behavior: TimeOutBehavior,

    _marker: PhantomData<E>,
}

impl<C, E> StatusEffectPlugin<C, E> {
    pub fn new(config: StatusEffectApplication<C>) -> Self { ... }

    pub fn with_timeout_behavior(mut self, behavior: TimeOutBehavior) -> Self {
        self.timeout_behavior = behavior;
        self
    }
}
```

---

## System Flow

### Applying an Effect

```
1. User triggers: ApplyStatusEffect { effect, duration, source }

2. Observer handles:
   a. Get or insert ActiveEffects<E> component on target
   b. Apply effect to component: effect.apply(&mut component, power)
   c. If timeout_behavior == Additive:
      - Store applied_delta = new_value - old_value
   d. Create EffectInstance { modifier, duration, applied_delta, source }
   e. Push to ActiveEffects.instances vec
```

### Ticking Durations

```
1. FixedUpdate system iterates all ActiveEffects<E>

2. For each instance with Some(duration):
   a. Tick the timer
   b. If timer.finished():
      - Reverse the effect based on TimeOutBehavior
      - Fire EffectExpired<E> event
      - Mark instance for removal

3. Remove expired instances from vec
```

### Effect Reversal on Timeout

```rust
fn reverse_effect<C, E>(
    component: &mut C,
    instance: &EffectInstance,
    timeout_behavior: TimeOutBehavior,
    power: f32,
) where
    C: MutableComponent,
    E: StatusEffectApplicator<C>,
{
    match timeout_behavior {
        TimeOutBehavior::Multiplicative => {
            // Apply inverse operation
            let inverse = instance.modifier.inverse();
            // Use the applicator's apply method with inverse modifier
            inverse.apply_scaled(current_value, power);
        }
        TimeOutBehavior::Additive => {
            // Subtract the stored delta
            if let Some(delta) = instance.applied_delta {
                current_value -= delta;
            }
        }
    }
}
```

### ValueModifier Inverse

```rust
impl ValueModifier {
    /// Returns the inverse modifier for reversal.
    pub fn inverse(&self) -> Self {
        match self {
            // +10 → -10
            Self::Val(v) => Self::Val(-v),
            // +10% (1.1x) → inverse percentage
            // To reverse 1.1x, we need 1/1.1 = 0.909x = -9.09%
            Self::Percent(p) => {
                let multiplier = 1.0 + p / 100.0;
                let inverse_multiplier = 1.0 / multiplier;
                Self::Percent((inverse_multiplier - 1.0) * 100.0)
            }
        }
    }
}
```

---

## API Examples

### Permanent Effect (Existing Pattern)

```rust
// These are equivalent - permanent by default
commands.trigger_targets(
    ApplyStatusEffect::from(effect::WalkSpeed(ValueModifier::Percent(20.0))),
    entity
);

// Or explicitly
commands.trigger_targets(
    ApplyStatusEffect::permanent(effect::WalkSpeed(ValueModifier::Percent(20.0))),
    entity
);
```

### Timed Effect

```rust
// 30-second speed buff
commands.trigger_targets(
    ApplyStatusEffect::timed(
        effect::WalkSpeed(ValueModifier::Percent(20.0)),
        Duration::from_secs(30)
    ),
    entity
);

// Or using builder pattern
commands.trigger_targets(
    ApplyStatusEffect::from(effect::WalkSpeed(ValueModifier::Percent(20.0)))
        .with_duration(Duration::from_secs(30))
        .from_source(buff_shrine),
    entity
);
```

### Querying Active Effects

```rust
fn display_speed_buffs(
    q_actors: Query<&ActiveEffects<effect::WalkSpeed>>,
) {
    for effects in &q_actors {
        for instance in &effects.instances {
            let duration_str = match &instance.duration {
                Some(timer) => format!("{:.1}s", timer.remaining_secs()),
                None => "permanent".to_string(),
            };
            println!(
                "Speed: {} ({})",
                instance.modifier.label(),
                duration_str
            );
        }
    }
}
```

### Listening for Expiration

```rust
fn on_speed_buff_expired(
    trigger: Trigger<EffectExpired<effect::WalkSpeed>>,
) {
    let entity = trigger.target();
    info!("Speed buff expired on {:?}", entity);
}
```

### Removing Effects

```rust
// Remove all speed effects from entity
commands.trigger_targets(RemoveAllEffects::<effect::WalkSpeed>::new(), entity);

// Remove only effects from a specific source
commands.trigger_targets(
    RemoveEffectsFromSource::<effect::WalkSpeed>::new(buff_shrine),
    entity
);
```

---

## TimeOutBehavior Examples

### Multiplicative (Default)

```
Initial speed: 100
Apply +20% buff → speed becomes 120
... player picks up permanent +50 speed item → speed becomes 170
Buff expires, apply -20% → speed becomes 136 (170 * 0.833)

Note: Player "loses" more than they gained because the percentage
applies to the current (higher) value. This is intentional and can
create interesting gameplay dynamics.
```

### Additive

```
Initial speed: 100
Apply +20% buff → speed becomes 120, store delta = +20
... player picks up permanent +50 speed item → speed becomes 170
Buff expires, subtract stored delta → speed becomes 150 (170 - 20)

Note: Player loses exactly what they gained, regardless of other
changes. This is the "fair" approach.
```

---

## Module Structure

```
msg_status_effect/
├── src/
│   ├── lib.rs              # Re-exports, prelude
│   ├── modifier.rs         # ValueModifier + inverse()
│   ├── scaling.rs          # Power scaling constants
│   ├── applicator.rs       # StatusEffectApplicator trait
│   ├── effects/
│   │   ├── mod.rs
│   │   ├── instance.rs     # EffectInstance struct
│   │   ├── active.rs       # ActiveEffects<E> component
│   │   ├── timeout.rs      # TimeOutBehavior enum
│   │   └── category.rs     # EffectCategory enum
│   ├── events/
│   │   ├── mod.rs
│   │   ├── apply.rs        # ApplyStatusEffect
│   │   ├── expired.rs      # EffectExpired
│   │   └── remove.rs       # RemoveAllEffects, RemoveEffectsFromSource
│   ├── systems/
│   │   ├── mod.rs
│   │   ├── apply.rs        # Observer for ApplyStatusEffect
│   │   ├── tick.rs         # Duration ticking system
│   │   ├── expire.rs       # Expiration handling
│   │   └── remove.rs       # Manual removal observers
│   └── plugin.rs           # StatusEffectPlugin
└── Cargo.toml
```

---

## Backwards Compatibility

The existing API works unchanged:

```rust
// Before (still works - permanent effect)
commands.trigger_targets(
    ApplyStatusEffect(effect::WalkSpeed(ValueModifier::Percent(20.0))),
    entity
);

// After (new capability - timed effect)
commands.trigger_targets(
    ApplyStatusEffect(effect::WalkSpeed(ValueModifier::Percent(20.0)))
        .with_duration(Duration::from_secs(30)),
    entity
);
```

---

## Implementation Steps

### Phase 1: Core Data Structures
1. Add `EffectInstance` struct
2. Add `ActiveEffects<E>` component
3. Add `TimeOutBehavior` enum
4. Implement `ValueModifier::inverse()`

### Phase 2: Extended Events
1. Extend `ApplyStatusEffect` with duration/source fields
2. Add builder methods (with_duration, from_source)
3. Add `EffectExpired<E>` event
4. Add `RemoveAllEffects<E>`, `RemoveEffectsFromSource<E>` events

### Phase 3: Systems
1. Modify apply observer to create EffectInstance and store in ActiveEffects
2. Add duration tick system (FixedUpdate)
3. Add expiration handler with TimeOutBehavior logic
4. Add removal observers

### Phase 4: Plugin Integration
1. Update `StatusEffectPlugin` with timeout_behavior config
2. Ensure backwards compatibility
3. Add `EffectCategory` support

### Phase 5: Testing
1. Unit tests for ValueModifier::inverse()
2. Integration tests for both TimeOutBehavior modes
3. Stacking tests (multiple instances)
4. Source-based removal tests

### Phase 6: Documentation
1. Update crate docs with new API
2. Add examples for common patterns
3. Document TimeOutBehavior tradeoffs

---

## Testing Strategy

### Unit Tests
- `ValueModifier::inverse()` correctness
- `EffectInstance` creation and storage
- `TimeOutBehavior` reversal math

### Integration Tests
- Apply permanent → verify no expiration
- Apply timed → verify expiration after duration
- Apply multiple → verify all tracked in vec
- Multiplicative timeout → verify inverse applied
- Additive timeout → verify exact delta subtracted
- Remove by source → verify only matching removed

### Edge Cases
- Zero duration (immediate expiration)
- Apply effect to entity without component (auto-insert)
- Source entity despawned before effect expires
- Overlapping effects with different sources

---

## Open Questions (Resolved)

1. ~~Should timed effects also call `apply()` to mutate component?~~
   **Yes** - Effects always mutate the component immediately. TimeOutBehavior controls reversal.

2. ~~How to handle pause state?~~
   **Use Bevy's Time resource** - Effects tick with virtual time, respecting pause.

3. ~~Stacking behavior?~~
   **Vec-based** - All instances stack. Games can implement custom stacking by querying ActiveEffects before applying.
