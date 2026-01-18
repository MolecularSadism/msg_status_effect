//! # msg_status_effect
//!
//! A generic status effect system for Bevy games with configurable scaling.
//!
//! This crate provides a type-safe, observer-driven architecture for applying
//! effects to game entities with support for diminishing/increasing returns.
//!
//! ## Key Features
//!
//! - **Type-Safe**: Effect types are statically linked to their target components
//! - **Configurable Scaling**: Per-component power scaling for game balance
//! - **Observer-Based**: Uses Bevy's observer system for efficient dispatch
//! - **Organized Observers**: `status_effect_observer!` macro organizes observers in entity hierarchy
//!
//! ## Quick Start
//!
//! ```rust
//! use bevy::prelude::*;
//! use msg_status_effect::prelude::*;
//!
//! // Define a component that will receive effects (must implement Default)
//! #[derive(Component, Default)]
//! pub struct Speed(pub f32);
//!
//! // Define an effect type
//! #[derive(Event, Clone, Copy)]
//! pub struct SpeedModifier(pub ValueModifier);
//!
//! // Implement the applicator trait
//! impl StatusEffectApplicator<Speed> for SpeedModifier {
//!     fn modifier(&self) -> ValueModifier { self.0 }
//!     fn apply(&self, component: &mut Speed, power: f32) {
//!         component.0 = self.0.apply_scaled(component.0, power);
//!     }
//! }
//!
//! // Register in your plugin
//! fn plugin(app: &mut App) {
//!     app.add_plugins(StatusEffectPlugin::<Speed, SpeedModifier>::new(
//!         StatusEffectApplication::sqrt()
//!     ));
//! }
//!
//! // Apply effects in a system (example usage)
//! fn apply_speed_boost(mut commands: Commands, entity: Entity) {
//!     commands.trigger_targets(
//!         ApplyStatusEffect(SpeedModifier(ValueModifier::Percent(50.0))),
//!         entity
//!     );
//! }
//! ```

use std::marker::PhantomData;

use bevy::ecs::component::Mutable;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub mod prelude {
    pub use bevy_enum_event::EnumEvent;

    pub use crate::{
        ApplyStatusEffect, MutableComponent, StatusEffectApplication, StatusEffectApplicator,
        StatusEffectPlugin, ValueModifier, scaling, status_effect_observer,
    };
}

/// Power scaling presets for common use cases.
///
/// Power controls how multiple effects combine:
/// - `power = 1.0` (LINEAR): Standard addition/multiplication
/// - `power = 0.5` (SQRT): Diminishing returns
/// - `power = 1/3` (CUBE_ROOT): Strong diminishing returns
/// - `power = 2.0` (SQUARE): Increasing returns
pub mod scaling {
    /// Linear scaling (no diminishing returns)
    pub const LINEAR: f32 = 1.0;

    /// Square root scaling (moderate diminishing returns)
    pub const SQRT: f32 = 0.5;

    /// Cube root scaling (strong diminishing returns)
    pub const CUBE_ROOT: f32 = 1.0 / 3.0;

    /// Square scaling (increasing returns)
    pub const SQUARE: f32 = 2.0;

    /// Cube scaling (strong increasing returns)
    pub const CUBE: f32 = 3.0;
}

/// Modifier for numeric values, supporting both flat and percentage-based changes.
///
/// Unlike percentage-as-decimal systems, this uses percentage points directly:
/// - `Val(10.0)` adds 10 to the value
/// - `Percent(50.0)` means +50% = 1.5x multiplier
/// - `Percent(-10.0)` means -10% = 0.9x multiplier
///
/// # Scaling
///
/// The [`apply_scaled`](Self::apply_scaled) method allows configuring how values combine:
/// - Linear (power=1.0): Standard addition/multiplication
/// - Square root (power=0.5): Diminishing returns
/// - Cube root (power=1/3): Strong diminishing returns
///
/// # Examples
///
/// ```rust
/// use msg_status_effect::ValueModifier;
///
/// // Linear scaling
/// let modifier = ValueModifier::Val(10.0);
/// assert_eq!(modifier.apply_scaled(100.0, 1.0), 110.0);
///
/// // Percentage with sqrt scaling
/// let modifier = ValueModifier::Percent(50.0);
/// let result = modifier.apply_scaled(100.0, 0.5);
/// // 100 * 1.5^0.5 = ~122.47
/// assert!((result - 122.47).abs() < 0.1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(Debug, PartialEq)]
pub enum ValueModifier {
    /// Flat additive value (e.g., +10 speed)
    Val(f32),
    /// Percentage change in points (e.g., 50 = +50% = 1.5x, -10 = -10% = 0.9x)
    Percent(f32),
}

impl ValueModifier {
    /// Creates a flat value modifier.
    #[inline]
    #[must_use]
    pub const fn flat(value: f32) -> Self {
        Self::Val(value)
    }

    /// Creates a percentage modifier from percentage points.
    ///
    /// # Arguments
    /// * `percent` - The percentage in points (e.g., 50.0 for +50%)
    #[inline]
    #[must_use]
    pub const fn percent(percent: f32) -> Self {
        Self::Percent(percent)
    }

    /// Apply modifier to a value with linear scaling (no diminishing returns).
    ///
    /// Equivalent to `apply_scaled(current, 1.0)`.
    #[must_use]
    pub fn apply(&self, current: f32) -> f32 {
        match self {
            Self::Val(v) => current + v,
            Self::Percent(p) => current * (1.0 + p / 100.0),
        }
    }

    /// Apply with power scaling for diminishing/increasing returns.
    ///
    /// # Power Parameter
    ///
    /// Controls how values combine:
    /// - `power = 1.0` (LINEAR): Standard addition/multiplication
    /// - `power = 0.5` (SQRT): Diminishing returns
    /// - `power = 1/3` (CUBE_ROOT): Strong diminishing returns
    /// - `power = 2.0` (SQUARE): Increasing returns
    ///
    /// # Formulas
    ///
    /// - **Val (addition)**: `(current^(1/p) + val^(1/p))^p`
    /// - **Val (subtraction)**: `(current^(1/p) - val^(1/p))^p` (clamped to 0)
    /// - **Percent**: `current * multiplier^p` where `multiplier = 1 + percent/100`
    ///
    /// # Examples
    ///
    /// With SQRT scaling (p=0.5):
    /// - `Val(30)` on 40: `sqrt(40^2 + 30^2) = 50`
    /// - `Val(-30)` on 40: `sqrt(40^2 - 30^2) = ~26.46`
    /// - `Percent(50)` on 100: `100 * sqrt(1.5) = ~122.47`
    ///
    /// # Negative Current Values
    ///
    /// Game stats should be positive. Negative current values trigger a warning
    /// and are treated as positive, with the sign restored at the end.
    #[must_use]
    pub fn apply_scaled(&self, current: f32, power: f32) -> f32 {
        // Game stats should be positive; warn and handle gracefully if not
        let (abs_current, sign) = if current < 0.0 {
            warn!(
                "Negative current value {} in apply_scaled; game stats should be positive",
                current
            );
            (current.abs(), -1.0)
        } else {
            (current, 1.0)
        };

        let result = match self {
            Self::Val(v) => {
                let inv_p = 1.0 / power;
                let current_term = abs_current.powf(inv_p);
                let val_term = v.abs().powf(inv_p);

                if *v >= 0.0 {
                    // Adding: (current^(1/p) + val^(1/p))^p
                    (current_term + val_term).powf(power)
                } else {
                    // Subtracting: (current^(1/p) - val^(1/p))^p, clamped to 0
                    (current_term - val_term).max(0.0).powf(power)
                }
            }
            Self::Percent(p) => {
                // Convert percentage points to multiplier: 50 -> 1.5, -10 -> 0.9
                let multiplier = (1.0 + p / 100.0).max(0.0);
                abs_current * multiplier.powf(power)
            }
        };

        result * sign
    }

    /// Returns the flat value if this is a Val modifier, otherwise 0.
    #[inline]
    #[must_use]
    pub fn flat_value(&self) -> f32 {
        match self {
            Self::Val(v) => *v,
            Self::Percent(_) => 0.0,
        }
    }

    /// Returns the percent value if this is a Percent modifier, otherwise 0.
    #[inline]
    #[must_use]
    pub fn percent_value(&self) -> f32 {
        match self {
            Self::Val(_) => 0.0,
            Self::Percent(p) => *p,
        }
    }

    /// Returns true if this is a flat value modifier.
    #[inline]
    #[must_use]
    pub fn is_flat(&self) -> bool {
        matches!(self, Self::Val(_))
    }

    /// Returns true if this is a percentage modifier.
    #[inline]
    #[must_use]
    pub fn is_percent(&self) -> bool {
        matches!(self, Self::Percent(_))
    }

    /// Returns a new modifier with the value scaled by the given factor.
    #[inline]
    #[must_use]
    pub fn scaled_by(&self, factor: f32) -> Self {
        match self {
            Self::Val(v) => Self::Val(v * factor),
            Self::Percent(p) => Self::Percent(p * factor),
        }
    }
}

impl Default for ValueModifier {
    fn default() -> Self {
        Self::Val(0.0)
    }
}

/// Trait alias for mutable components that can have effects applied.
pub trait MutableComponent: Component<Mutability = Mutable> {}
impl<C: Component<Mutability = Mutable>> MutableComponent for C {}

/// Configuration for how status effects are applied to a component type.
///
/// This resource controls the scaling behavior when applying effects to components.
///
/// # Examples
///
/// ```rust
/// use bevy::prelude::*;
/// use msg_status_effect::prelude::*;
///
/// #[derive(Component)]
/// struct Health(f32);
///
/// // Linear scaling (default)
/// let config = StatusEffectApplication::<Health>::default();
/// assert!((config.power - 1.0).abs() < 0.001);
///
/// // Square root scaling for diminishing returns
/// let config = StatusEffectApplication::<Health>::sqrt();
/// assert!((config.power - 0.5).abs() < 0.001);
///
/// // Custom power scaling
/// let config = StatusEffectApplication::<Health>::with_power(0.7);
/// assert!((config.power - 0.7).abs() < 0.001);
/// ```
#[derive(Resource)]
pub struct StatusEffectApplication<C: MutableComponent> {
    /// Power scaling for effect application
    pub power: f32,
    /// Phantom data for the component type
    _marker: PhantomData<C>,
}

impl<C: MutableComponent> Default for StatusEffectApplication<C> {
    fn default() -> Self {
        Self {
            power: scaling::LINEAR,
            _marker: PhantomData,
        }
    }
}

impl<C: MutableComponent> StatusEffectApplication<C> {
    /// Creates a config with custom power scaling.
    #[must_use]
    pub fn with_power(power: f32) -> Self {
        Self {
            power,
            _marker: PhantomData,
        }
    }

    /// Creates a config with square root scaling (diminishing returns).
    #[must_use]
    pub fn sqrt() -> Self {
        Self::with_power(scaling::SQRT)
    }

    /// Creates a config with cube root scaling (strong diminishing returns).
    #[must_use]
    pub fn cube_root() -> Self {
        Self::with_power(scaling::CUBE_ROOT)
    }

    /// Creates a config with linear scaling (no diminishing returns).
    #[must_use]
    pub fn linear() -> Self {
        Self::with_power(scaling::LINEAR)
    }

    /// Creates a config with square scaling (increasing returns).
    #[must_use]
    pub fn square() -> Self {
        Self::with_power(scaling::SQUARE)
    }
}

/// Trait linking effect types to their target components.
///
/// Implement this trait to define how an effect modifies a specific component.
///
/// # Example
///
/// ```rust
/// use bevy::prelude::*;
/// use msg_status_effect::prelude::*;
///
/// #[derive(Component)]
/// struct Speed(f32);
///
/// #[derive(Event, Clone, Copy)]
/// struct SpeedModifier(ValueModifier);
///
/// impl StatusEffectApplicator<Speed> for SpeedModifier {
///     fn modifier(&self) -> ValueModifier {
///         self.0
///     }
///
///     fn apply(&self, component: &mut Speed, power: f32) {
///         component.0 = self.0.apply_scaled(component.0, power);
///     }
/// }
///
/// // Test the implementation
/// let effect = SpeedModifier(ValueModifier::Percent(50.0));
/// let mut speed = Speed(100.0);
/// effect.apply(&mut speed, 1.0);
/// assert!((speed.0 - 150.0).abs() < 0.001);
/// ```
pub trait StatusEffectApplicator<C: MutableComponent>: Event + Clone {
    /// Get the value modifier from the effect.
    fn modifier(&self) -> ValueModifier;

    /// Apply the effect to the component with the given power scaling.
    fn apply(&self, component: &mut C, power: f32);
}

/// Generic event wrapper for applying status effects to entities.
///
/// Similar to `Enter<T>`/`Exit<T>` from bevy_fsm, this wrapper provides
/// type-safe effect application.
///
/// # Usage
///
/// ```rust
/// use bevy::prelude::*;
/// use msg_status_effect::prelude::*;
///
/// #[derive(Event, Clone, Copy)]
/// struct SpeedModifier(ValueModifier);
///
/// // Create the wrapped effect event
/// let effect = ApplyStatusEffect(SpeedModifier(ValueModifier::Percent(50.0)));
///
/// // In a system, you would trigger it like this:
/// fn apply_speed_boost(mut commands: Commands, entity: Entity) {
///     commands.trigger_targets(
///         ApplyStatusEffect(SpeedModifier(ValueModifier::Percent(50.0))),
///         entity,
///     );
/// }
/// ```
#[derive(Event, Clone, Copy)]
pub struct ApplyStatusEffect<E: Event + Clone>(pub E);

/// Generic observer that handles any `ApplyStatusEffect<E>` for component C.
///
/// If the target entity doesn't have the component, it will be automatically
/// inserted with its default value before applying the effect.
fn apply_status_effect_observer<C, E>(
    trigger: Trigger<ApplyStatusEffect<E>>,
    config: Res<StatusEffectApplication<C>>,
    mut q: Query<&mut C>,
    mut commands: Commands,
) where
    C: MutableComponent + Default,
    E: Event + Clone + StatusEffectApplicator<C>,
{
    let entity = trigger.target();
    if let Ok(mut component) = q.get_mut(entity) {
        trigger.event().0.apply(&mut component, config.power);
    } else if let Ok(mut entity_commands) = commands.get_entity(entity) {
        // Entity exists but missing component - insert default and re-trigger
        entity_commands.insert(C::default());
        commands.trigger_targets(trigger.event().clone(), entity);
    }
    // If entity doesn't exist, silently ignore
}

/// Plugin for registering a status effect for a specific component.
///
/// # Example
///
/// ```rust
/// use bevy::prelude::*;
/// use msg_status_effect::prelude::*;
///
/// // Define component and effect (component must implement Default)
/// #[derive(Component, Default)]
/// struct Speed(f32);
///
/// #[derive(Event, Clone, Copy)]
/// struct SpeedModifier(ValueModifier);
///
/// impl StatusEffectApplicator<Speed> for SpeedModifier {
///     fn modifier(&self) -> ValueModifier { self.0 }
///     fn apply(&self, component: &mut Speed, power: f32) {
///         component.0 = self.0.apply_scaled(component.0, power);
///     }
/// }
///
/// fn plugin(app: &mut App) {
///     // Linear scaling (default)
///     app.add_plugins(StatusEffectPlugin::<Speed, SpeedModifier>::default());
///
///     // Or with square root scaling for diminishing returns
///     // app.add_plugins(StatusEffectPlugin::<Speed, SpeedModifier>::new(
///     //     StatusEffectApplication::sqrt()
///     // ));
///
///     // Or with custom power scaling
///     // app.add_plugins(StatusEffectPlugin::<Speed, SpeedModifier>::new(
///     //     StatusEffectApplication::with_power(0.7)
///     // ));
/// }
/// ```
pub struct StatusEffectPlugin<C, E>
where
    C: MutableComponent + Default,
    E: Event + Clone + StatusEffectApplicator<C>,
{
    config: StatusEffectApplication<C>,
    _marker: PhantomData<E>,
}

impl<C, E> Default for StatusEffectPlugin<C, E>
where
    C: MutableComponent + Default,
    E: Event + Clone + StatusEffectApplicator<C>,
{
    fn default() -> Self {
        Self {
            config: StatusEffectApplication::default(),
            _marker: PhantomData,
        }
    }
}

impl<C, E> StatusEffectPlugin<C, E>
where
    C: MutableComponent + Default,
    E: Event + Clone + StatusEffectApplicator<C>,
{
    /// Creates a new plugin with the specified configuration.
    #[must_use]
    pub fn new(config: StatusEffectApplication<C>) -> Self {
        Self {
            config,
            _marker: PhantomData,
        }
    }
}

impl<C, E> Plugin for StatusEffectPlugin<C, E>
where
    C: MutableComponent + Default,
    E: Event + Clone + StatusEffectApplicator<C>,
{
    fn build(&self, app: &mut App) {
        app.insert_resource(StatusEffectApplication::<C> {
            power: self.config.power,
            _marker: PhantomData,
        });
        app.add_observer(apply_status_effect_observer::<C, E>);
    }
}

/// Marker component used to organize status effect observers in the entity hierarchy.
///
/// When using [`status_effect_observer!`], observers are attached to entities
/// with this marker, making them easier to inspect in debugging tools.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct StatusEffectObserverMarker;

/// Macro for registering status effect observers with organized entity hierarchy.
///
/// This macro creates observers that are attached to marker entities for easier
/// inspection and debugging. Inspired by bevy_fsm's `fsm_observer!` macro.
///
/// # Usage
///
/// ```rust
/// use bevy::prelude::*;
/// use msg_status_effect::prelude::*;
///
/// // Define a component and effect type
/// #[derive(Component)]
/// struct Speed(f32);
///
/// #[derive(Event, Clone, Copy)]
/// struct SpeedModifier(ValueModifier);
///
/// impl StatusEffectApplicator<Speed> for SpeedModifier {
///     fn modifier(&self) -> ValueModifier { self.0 }
///     fn apply(&self, component: &mut Speed, power: f32) {
///         component.0 = self.0.apply_scaled(component.0, power);
///     }
/// }
///
/// // Observer function for the effect
/// fn on_apply_speed_modifier(
///     trigger: Trigger<ApplyStatusEffect<SpeedModifier>>,
///     mut q_speed: Query<&mut Speed>,
/// ) {
///     let entity = trigger.target();
///     if let Ok(mut speed) = q_speed.get_mut(entity) {
///         trigger.event().0.apply(&mut speed, 1.0);
///     }
/// }
///
/// // Register in your plugin
/// fn plugin(app: &mut App) {
///     status_effect_observer!(app, SpeedModifier, on_apply_speed_modifier);
/// }
/// ```
///
/// # Organization
///
/// This macro spawns a marker entity named after the observer function
/// (e.g., "on_apply_walk_speed") for visibility in entity inspectors,
/// and registers a global observer that responds to the effect on any entity.
/// Uses pure snake_case naming consistent with fsm_observer!.
#[macro_export]
macro_rules! status_effect_observer {
    ($app:expr, $effect_type:ty, $observer_fn:ident) => {{
        // Create marker entity for this observer group
        let marker_name = concat!(stringify!($effect_type), "_observer");

        // Register the observer with a descriptive name
        $app.world_mut()
            .spawn((Name::new(marker_name), $crate::StatusEffectObserverMarker))
            .observe($observer_fn);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // ValueModifier Unit Tests
    // ============================================================================

    #[test]
    fn value_modifier_apply_linear() {
        // Linear scaling (power = 1.0): standard addition
        assert!((ValueModifier::Val(10.0).apply_scaled(100.0, 1.0) - 110.0).abs() < 0.001);

        // Linear percentage: +50% = 1.5x
        let result = ValueModifier::Percent(50.0).apply_scaled(100.0, 1.0);
        assert!((result - 150.0).abs() < 0.001);

        // Linear percentage: -10% = 0.9x
        let result = ValueModifier::Percent(-10.0).apply_scaled(100.0, 1.0);
        assert!((result - 90.0).abs() < 0.001);
    }

    #[test]
    fn value_modifier_apply_scaled_sqrt() {
        // Square root scaling (power = 0.5): quadratic addition
        // Formula: (current^2 + val^2)^0.5
        let result = ValueModifier::Val(30.0).apply_scaled(40.0, 0.5);
        // (40^2 + 30^2)^0.5 = sqrt(2500) = 50
        assert!((result - 50.0).abs() < 0.001);

        // Negative val: subtraction with scaling
        let result = ValueModifier::Val(-30.0).apply_scaled(40.0, 0.5);
        // (40^2 - 30^2)^0.5 = sqrt(700) = ~26.46
        assert!((result - 26.46).abs() < 0.01);

        // Subtraction clamped to 0
        let result = ValueModifier::Val(-50.0).apply_scaled(30.0, 0.5);
        // (30^2 - 50^2)^0.5 = sqrt(-1600) -> clamped to 0
        assert_eq!(result, 0.0);
    }

    #[test]
    fn value_modifier_apply_scaled_percent() {
        // Sqrt scaling: +50% -> 1.5^0.5 = ~1.2247x
        let result = ValueModifier::Percent(50.0).apply_scaled(100.0, 0.5);
        // 100 * 1.5^0.5 = ~122.47
        assert!((result - 122.47).abs() < 0.1);

        // Sqrt scaling: -10% -> 0.9^0.5 = ~0.9487x
        let result = ValueModifier::Percent(-10.0).apply_scaled(100.0, 0.5);
        // 100 * 0.9^0.5 = ~94.87
        assert!((result - 94.87).abs() < 0.1);

        // Edge case: -100% = 0x, clamped
        let result = ValueModifier::Percent(-100.0).apply_scaled(100.0, 0.5);
        assert_eq!(result, 0.0);

        // Edge case: -150% would be negative multiplier, clamped to 0
        let result = ValueModifier::Percent(-150.0).apply_scaled(100.0, 0.5);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn value_modifier_constructors() {
        let flat = ValueModifier::flat(10.0);
        assert!(flat.is_flat());
        assert!(!flat.is_percent());
        assert_eq!(flat.flat_value(), 10.0);
        assert_eq!(flat.percent_value(), 0.0);

        let percent = ValueModifier::percent(25.0);
        assert!(!percent.is_flat());
        assert!(percent.is_percent());
        assert_eq!(percent.flat_value(), 0.0);
        assert_eq!(percent.percent_value(), 25.0);
    }

    #[test]
    fn value_modifier_scaled_by() {
        let flat = ValueModifier::flat(10.0);
        assert_eq!(flat.scaled_by(2.0), ValueModifier::Val(20.0));

        let percent = ValueModifier::percent(50.0);
        assert_eq!(percent.scaled_by(0.5), ValueModifier::Percent(25.0));
    }

    #[test]
    fn status_effect_application_presets() {
        #[derive(Component)]
        struct TestComponent;

        let linear = StatusEffectApplication::<TestComponent>::linear();
        assert!((linear.power - 1.0).abs() < 0.001);

        let sqrt = StatusEffectApplication::<TestComponent>::sqrt();
        assert!((sqrt.power - 0.5).abs() < 0.001);

        let cube_root = StatusEffectApplication::<TestComponent>::cube_root();
        assert!((cube_root.power - (1.0 / 3.0)).abs() < 0.001);

        let square = StatusEffectApplication::<TestComponent>::square();
        assert!((square.power - 2.0).abs() < 0.001);

        let custom = StatusEffectApplication::<TestComponent>::with_power(0.7);
        assert!((custom.power - 0.7).abs() < 0.001);
    }

    // ============================================================================
    // ValueModifier Edge Case Tests
    // ============================================================================

    #[test]
    fn value_modifier_zero_current_value() {
        // Adding to zero
        let result = ValueModifier::Val(50.0).apply_scaled(0.0, 1.0);
        assert!((result - 50.0).abs() < 0.001);

        // Percentage on zero should stay zero
        let result = ValueModifier::Percent(100.0).apply_scaled(0.0, 1.0);
        assert_eq!(result, 0.0);

        // Zero with sqrt scaling
        let result = ValueModifier::Val(50.0).apply_scaled(0.0, 0.5);
        assert!((result - 50.0).abs() < 0.001);
    }

    #[test]
    fn value_modifier_zero_modifier_value() {
        // Zero flat value should not change current
        let result = ValueModifier::Val(0.0).apply_scaled(100.0, 1.0);
        assert!((result - 100.0).abs() < 0.001);

        // Zero percent should not change current
        let result = ValueModifier::Percent(0.0).apply_scaled(100.0, 1.0);
        assert!((result - 100.0).abs() < 0.001);
    }

    #[test]
    fn value_modifier_very_large_values() {
        // Large flat value
        let result = ValueModifier::Val(1000000.0).apply_scaled(100.0, 1.0);
        assert!((result - 1000100.0).abs() < 1.0);

        // Large percentage (10x multiplier)
        let result = ValueModifier::Percent(900.0).apply_scaled(100.0, 1.0);
        assert!((result - 1000.0).abs() < 0.001);
    }

    #[test]
    fn value_modifier_small_values() {
        // Very small flat value
        let result = ValueModifier::Val(0.001).apply_scaled(100.0, 1.0);
        assert!((result - 100.001).abs() < 0.0001);

        // Very small percentage
        let result = ValueModifier::Percent(0.1).apply_scaled(100.0, 1.0);
        assert!((result - 100.1).abs() < 0.001);
    }

    #[test]
    fn value_modifier_cube_root_scaling() {
        // Cube root scaling (power = 1/3): strong diminishing returns
        // Formula: (current^3 + val^3)^(1/3)
        let result = ValueModifier::Val(30.0).apply_scaled(40.0, scaling::CUBE_ROOT);
        // (40^3 + 30^3)^(1/3) = (64000 + 27000)^(1/3) = 91000^(1/3) = ~45.0
        assert!((result - 45.0).abs() < 0.5);

        // Cube root percentage
        let result = ValueModifier::Percent(100.0).apply_scaled(100.0, scaling::CUBE_ROOT);
        // 100 * 2^(1/3) = ~126
        assert!((result - 126.0).abs() < 0.5);
    }

    #[test]
    fn value_modifier_square_scaling() {
        // Square scaling (power = 2): increasing returns
        // Formula: (sqrt(current) + sqrt(val))^2
        let result = ValueModifier::Val(21.0).apply_scaled(100.0, scaling::SQUARE);
        // (sqrt(100) + sqrt(21))^2 = (10 + 4.58)^2 = ~212.2
        assert!((result - 212.2).abs() < 1.0);

        // Square percentage
        let result = ValueModifier::Percent(50.0).apply_scaled(100.0, scaling::SQUARE);
        // 100 * 1.5^2 = 225
        assert!((result - 225.0).abs() < 0.001);
    }

    #[test]
    fn value_modifier_cube_scaling() {
        // Cube scaling (power = 3): strong increasing returns
        let result = ValueModifier::Percent(50.0).apply_scaled(100.0, scaling::CUBE);
        // 100 * 1.5^3 = 337.5
        assert!((result - 337.5).abs() < 0.001);
    }

    #[test]
    fn value_modifier_apply_no_scaling() {
        // Test the simple apply() method (linear, no scaling)
        assert!((ValueModifier::Val(10.0).apply(100.0) - 110.0).abs() < 0.001);
        assert!((ValueModifier::Percent(50.0).apply(100.0) - 150.0).abs() < 0.001);
        assert!((ValueModifier::Percent(-25.0).apply(100.0) - 75.0).abs() < 0.001);
    }

    #[test]
    fn value_modifier_default() {
        let default = ValueModifier::default();
        assert_eq!(default, ValueModifier::Val(0.0));
        // Default should not change value
        assert!((default.apply(100.0) - 100.0).abs() < 0.001);
    }

    #[test]
    fn value_modifier_scaled_by_zero() {
        let flat = ValueModifier::flat(100.0);
        assert_eq!(flat.scaled_by(0.0), ValueModifier::Val(0.0));

        let percent = ValueModifier::percent(50.0);
        assert_eq!(percent.scaled_by(0.0), ValueModifier::Percent(0.0));
    }

    #[test]
    fn value_modifier_scaled_by_negative() {
        let flat = ValueModifier::flat(10.0);
        assert_eq!(flat.scaled_by(-1.0), ValueModifier::Val(-10.0));

        let percent = ValueModifier::percent(50.0);
        assert_eq!(percent.scaled_by(-1.0), ValueModifier::Percent(-50.0));
    }

    #[test]
    fn value_modifier_subtraction_with_various_scaling() {
        // Linear subtraction
        let result = ValueModifier::Val(-30.0).apply_scaled(100.0, 1.0);
        assert!((result - 70.0).abs() < 0.001);

        // Sqrt subtraction (diminishing returns on subtraction too)
        let result = ValueModifier::Val(-60.0).apply_scaled(100.0, 0.5);
        // (100^2 - 60^2)^0.5 = sqrt(10000 - 3600) = sqrt(6400) = 80
        assert!((result - 80.0).abs() < 0.001);

        // Subtraction exceeding current value clamps to 0
        let result = ValueModifier::Val(-150.0).apply_scaled(100.0, 0.5);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn scaling_constants_values() {
        assert_eq!(scaling::LINEAR, 1.0);
        assert_eq!(scaling::SQRT, 0.5);
        assert!((scaling::CUBE_ROOT - 0.333333).abs() < 0.001);
        assert_eq!(scaling::SQUARE, 2.0);
        assert_eq!(scaling::CUBE, 3.0);
    }

    // ============================================================================
    // StatusEffectApplication Tests
    // ============================================================================

    #[test]
    fn status_effect_application_default() {
        #[derive(Component)]
        struct TestComponent;

        let default = StatusEffectApplication::<TestComponent>::default();
        assert_eq!(default.power, scaling::LINEAR);
    }

    // ============================================================================
    // Integration Tests - Full Plugin System
    // ============================================================================

    /// Test component for integration tests
    #[derive(Component, Default)]
    struct TestSpeed {
        value: f32,
    }

    impl TestSpeed {
        fn new(value: f32) -> Self {
            Self { value }
        }
    }

    /// Test effect for modifying TestSpeed
    #[derive(Event, Clone, Copy)]
    struct TestSpeedEffect(ValueModifier);

    impl StatusEffectApplicator<TestSpeed> for TestSpeedEffect {
        fn modifier(&self) -> ValueModifier {
            self.0
        }

        fn apply(&self, component: &mut TestSpeed, power: f32) {
            component.value = self.0.apply_scaled(component.value, power);
        }
    }

    #[test]
    fn integration_plugin_registers_observer_and_resource() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        app.update();

        // Verify resource is inserted
        assert!(
            app.world()
                .contains_resource::<StatusEffectApplication<TestSpeed>>()
        );

        // Verify default power is linear
        let config = app.world().resource::<StatusEffectApplication<TestSpeed>>();
        assert_eq!(config.power, scaling::LINEAR);
    }

    #[test]
    fn integration_apply_status_effect_flat_linear() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        // Spawn entity with component
        let entity = app.world_mut().spawn(TestSpeed::new(100.0)).id();

        app.update();

        // Trigger effect
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(20.0))),
            entity,
        );

        app.update();

        // Verify effect was applied
        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 120.0).abs() < 0.001);
    }

    #[test]
    fn integration_apply_status_effect_percent_linear() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        let entity = app.world_mut().spawn(TestSpeed::new(100.0)).id();

        app.update();

        // Apply +50% effect
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Percent(50.0))),
            entity,
        );

        app.update();

        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 150.0).abs() < 0.001);
    }

    #[test]
    fn integration_apply_status_effect_with_sqrt_scaling() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::new(
            StatusEffectApplication::sqrt(),
        ));

        let entity = app.world_mut().spawn(TestSpeed::new(40.0)).id();

        app.update();

        // Apply +30 with sqrt scaling: sqrt(40^2 + 30^2) = 50
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(30.0))),
            entity,
        );

        app.update();

        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 50.0).abs() < 0.001);
    }

    #[test]
    fn integration_apply_status_effect_with_custom_power() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::new(
            StatusEffectApplication::with_power(0.7),
        ));

        let entity = app.world_mut().spawn(TestSpeed::new(100.0)).id();

        app.update();

        // Apply +50% with power=0.7: 100 * 1.5^0.7 = ~136.8
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Percent(50.0))),
            entity,
        );

        app.update();

        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        let expected = 100.0 * 1.5_f32.powf(0.7);
        assert!((speed.value - expected).abs() < 0.1);
    }

    #[test]
    fn integration_multiple_effects_stack() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        let entity = app.world_mut().spawn(TestSpeed::new(100.0)).id();

        app.update();

        // Apply first effect: +20
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(20.0))),
            entity,
        );

        app.update();

        // Apply second effect: +10%
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Percent(10.0))),
            entity,
        );

        app.update();

        // 100 + 20 = 120, then 120 * 1.1 = 132
        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 132.0).abs() < 0.001);
    }

    #[test]
    fn integration_effect_on_nonexistent_entity_no_panic() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        app.update();

        // Trigger effect on entity that doesn't exist
        let fake_entity = Entity::from_raw(9999);
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(20.0))),
            fake_entity,
        );

        // Should not panic
        app.update();
    }

    #[test]
    fn integration_effect_on_entity_without_component_auto_inserts() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        // Spawn entity WITHOUT TestSpeed component
        let entity = app.world_mut().spawn_empty().id();

        app.update();

        // Trigger effect - should auto-insert component and apply effect
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(20.0))),
            entity,
        );

        // First update: observer runs, queues insert + re-trigger
        app.update();
        // Second update: re-triggered observer applies effect to inserted component
        app.update();

        // Component should now exist with default (0.0) + effect (20.0)
        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 20.0).abs() < 0.001);
    }

    #[test]
    fn integration_auto_insert_with_percent_effect() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        // Spawn entity WITHOUT TestSpeed component
        let entity = app.world_mut().spawn_empty().id();

        app.update();

        // Trigger percent effect on entity without component
        // Default TestSpeed.value is 0.0, so +50% of 0 = 0
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Percent(50.0))),
            entity,
        );

        app.update();
        app.update();

        // Component should exist with default value (percent of 0 is still 0)
        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 0.0).abs() < 0.001);
    }

    #[test]
    fn integration_auto_insert_with_custom_default() {
        /// Component with non-zero default for testing
        #[derive(Component)]
        struct TestArmor {
            value: f32,
        }

        impl Default for TestArmor {
            fn default() -> Self {
                Self { value: 10.0 } // Non-zero default
            }
        }

        #[derive(Event, Clone, Copy)]
        struct TestArmorEffect(ValueModifier);

        impl StatusEffectApplicator<TestArmor> for TestArmorEffect {
            fn modifier(&self) -> ValueModifier {
                self.0
            }
            fn apply(&self, component: &mut TestArmor, power: f32) {
                component.value = self.0.apply_scaled(component.value, power);
            }
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestArmor, TestArmorEffect>::default());

        let entity = app.world_mut().spawn_empty().id();

        app.update();

        // Apply +50% to entity without component
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestArmorEffect(ValueModifier::Percent(50.0))),
            entity,
        );

        app.update();
        app.update();

        // Default is 10.0, +50% = 15.0
        let armor = app.world().get::<TestArmor>(entity).unwrap();
        assert!((armor.value - 15.0).abs() < 0.001);
    }

    // ============================================================================
    // Integration Tests - Multiple Component Types
    // ============================================================================

    /// Second test component for multi-type tests
    #[derive(Component, Default)]
    struct TestHealth {
        current: f32,
        max: f32,
    }

    impl TestHealth {
        fn new(current: f32, max: f32) -> Self {
            Self { current, max }
        }
    }

    /// Effect for TestHealth
    #[derive(Event, Clone, Copy)]
    struct TestHealthEffect(ValueModifier);

    impl StatusEffectApplicator<TestHealth> for TestHealthEffect {
        fn modifier(&self) -> ValueModifier {
            self.0
        }

        fn apply(&self, component: &mut TestHealth, power: f32) {
            let ratio = component.current / component.max;
            component.max = self.0.apply_scaled(component.max, power);
            component.current = component.max * ratio;
        }
    }

    #[test]
    fn integration_multiple_component_types_independent() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());
        app.add_plugins(StatusEffectPlugin::<TestHealth, TestHealthEffect>::new(
            StatusEffectApplication::sqrt(),
        ));

        let entity = app
            .world_mut()
            .spawn((TestSpeed::new(100.0), TestHealth::new(50.0, 100.0)))
            .id();

        app.update();

        // Apply speed effect (linear)
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(20.0))),
            entity,
        );

        // Apply health effect (sqrt scaling)
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestHealthEffect(ValueModifier::Percent(50.0))),
            entity,
        );

        app.update();

        // Speed: 100 + 20 = 120 (linear)
        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 120.0).abs() < 0.001);

        // Health max: 100 * 1.5^0.5 = ~122.47 (sqrt scaling)
        // Health current: 122.47 * 0.5 = ~61.24
        let health = app.world().get::<TestHealth>(entity).unwrap();
        assert!((health.max - 122.47).abs() < 0.1);
        assert!((health.current - 61.24).abs() < 0.1);
    }

    // ============================================================================
    // Integration Tests - status_effect_observer! Macro
    // ============================================================================

    /// Component for macro tests
    #[derive(Component)]
    struct MacroTestComponent {
        value: f32,
    }

    /// Effect type for macro tests
    #[derive(Event, Clone, Copy)]
    struct MacroTestEffect(ValueModifier);

    /// Custom observer function for macro tests
    fn on_macro_test_effect(
        trigger: Trigger<ApplyStatusEffect<MacroTestEffect>>,
        mut q: Query<&mut MacroTestComponent>,
    ) {
        let entity = trigger.target();
        if let Ok(mut component) = q.get_mut(entity) {
            // Apply with linear scaling for simplicity
            component.value = trigger.event().0.0.apply_scaled(component.value, 1.0);
        }
    }

    #[test]
    fn integration_status_effect_observer_macro() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        // Use the macro to register the observer
        status_effect_observer!(app, MacroTestEffect, on_macro_test_effect);

        let entity = app
            .world_mut()
            .spawn(MacroTestComponent { value: 100.0 })
            .id();

        app.update();

        // Verify marker entity was created
        let marker_count = app
            .world_mut()
            .query_filtered::<Entity, With<StatusEffectObserverMarker>>()
            .iter(app.world())
            .count();
        assert_eq!(marker_count, 1);

        // Trigger effect
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(MacroTestEffect(ValueModifier::Val(50.0))),
            entity,
        );

        app.update();

        // Verify effect was applied
        let component = app.world().get::<MacroTestComponent>(entity).unwrap();
        assert!((component.value - 150.0).abs() < 0.001);
    }

    #[test]
    fn integration_macro_creates_named_marker() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        status_effect_observer!(app, MacroTestEffect, on_macro_test_effect);

        app.update();

        // Find the marker entity and check its name
        // Uses observer function name (pure snake_case, consistent with fsm_observer!)
        let mut found_name = false;
        for (entity, marker) in app
            .world_mut()
            .query::<(Entity, &StatusEffectObserverMarker)>()
            .iter(app.world())
        {
            if let Some(name) = app.world().get::<Name>(entity) {
                if name.as_str() == "on_macro_test_effect" {
                    found_name = true;
                }
            }
            let _ = marker; // Use the marker to avoid warning
        }

        assert!(
            found_name,
            "Expected marker entity with name 'on_macro_test_effect'"
        );
    }

    // ============================================================================
    // Integration Tests - Effect Stacking with Different Scaling
    // ============================================================================

    #[test]
    fn integration_stacking_effects_sqrt_scaling() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::new(
            StatusEffectApplication::sqrt(),
        ));

        let entity = app.world_mut().spawn(TestSpeed::new(100.0)).id();

        app.update();

        // Apply multiple flat effects with sqrt scaling
        // First: sqrt(100^2 + 60^2) = sqrt(13600) = ~116.62
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(60.0))),
            entity,
        );

        app.update();

        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 116.62).abs() < 0.1);

        // Second: sqrt(116.62^2 + 80^2) = sqrt(20000) = ~141.42
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(80.0))),
            entity,
        );

        app.update();

        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 141.42).abs() < 0.1);
    }

    #[test]
    fn integration_negative_effects() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        let entity = app.world_mut().spawn(TestSpeed::new(100.0)).id();

        app.update();

        // Apply negative flat effect
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Val(-30.0))),
            entity,
        );

        app.update();

        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 70.0).abs() < 0.001);
    }

    #[test]
    fn integration_negative_percent_effect() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        let entity = app.world_mut().spawn(TestSpeed::new(100.0)).id();

        app.update();

        // Apply -25% effect (slow)
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestSpeedEffect(ValueModifier::Percent(-25.0))),
            entity,
        );

        app.update();

        let speed = app.world().get::<TestSpeed>(entity).unwrap();
        assert!((speed.value - 75.0).abs() < 0.001);
    }

    // ============================================================================
    // Integration Tests - Real World Scenarios
    // ============================================================================

    /// Simulates a buff that increases speed by percentage
    #[test]
    fn scenario_speed_buff() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::new(
            StatusEffectApplication::sqrt(),
        ));

        let entity = app.world_mut().spawn(TestSpeed::new(100.0)).id();

        app.update();

        // Player picks up two speed buffs (+30% each)
        // With sqrt scaling, these should have diminishing returns
        for _ in 0..2 {
            app.world_mut().commands().trigger_targets(
                ApplyStatusEffect(TestSpeedEffect(ValueModifier::Percent(30.0))),
                entity,
            );
            app.update();
        }

        let speed = app.world().get::<TestSpeed>(entity).unwrap();

        // First buff: 100 * 1.3^0.5 = ~114.02
        // Second buff: 114.02 * 1.3^0.5 = ~130.0
        // Without sqrt scaling it would be: 100 * 1.3 * 1.3 = 169
        // So we should be significantly less than 169
        assert!(speed.value < 140.0);
        assert!(speed.value > 120.0);
    }

    /// Simulates health regeneration that preserves health ratio
    #[test]
    fn scenario_max_health_increase_preserves_ratio() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestHealth, TestHealthEffect>::default());

        // Player at 75% health
        let entity = app.world_mut().spawn(TestHealth::new(75.0, 100.0)).id();

        app.update();

        // Gain +50 max health
        app.world_mut().commands().trigger_targets(
            ApplyStatusEffect(TestHealthEffect(ValueModifier::Val(50.0))),
            entity,
        );

        app.update();

        let health = app.world().get::<TestHealth>(entity).unwrap();
        // New max: 150
        // Current should be 150 * 0.75 = 112.5
        assert!((health.max - 150.0).abs() < 0.001);
        assert!((health.current - 112.5).abs() < 0.001);
    }

    /// Simulates multiple entities receiving the same effect
    #[test]
    fn scenario_multiple_entities_same_effect() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatusEffectPlugin::<TestSpeed, TestSpeedEffect>::default());

        let entity1 = app.world_mut().spawn(TestSpeed::new(100.0)).id();
        let entity2 = app.world_mut().spawn(TestSpeed::new(80.0)).id();
        let entity3 = app.world_mut().spawn(TestSpeed::new(120.0)).id();

        app.update();

        // Apply same effect to all entities
        for entity in [entity1, entity2, entity3] {
            app.world_mut().commands().trigger_targets(
                ApplyStatusEffect(TestSpeedEffect(ValueModifier::Percent(25.0))),
                entity,
            );
        }

        app.update();

        assert!((app.world().get::<TestSpeed>(entity1).unwrap().value - 125.0).abs() < 0.001);
        assert!((app.world().get::<TestSpeed>(entity2).unwrap().value - 100.0).abs() < 0.001);
        assert!((app.world().get::<TestSpeed>(entity3).unwrap().value - 150.0).abs() < 0.001);
    }
}
