//! Basic usage example demonstrating the msg_status_effect crate.
//!
//! This example shows how to:
//! - Define components that can receive status effects
//! - Create effect types that modify those components
//! - Register the plugin with different scaling configurations
//! - Trigger effects on entities
//!
//! Run with: `cargo run --example basic_usage`

use bevy::prelude::*;
use msg_status_effect::prelude::*;

// ============================================================================
// Components that can receive status effects
// ============================================================================

/// A simple speed component representing movement speed.
/// Must implement Default for auto-insertion behavior.
#[derive(Component, Default)]
pub struct Speed {
    pub value: f32,
}

impl Speed {
    pub fn new(value: f32) -> Self {
        Self { value }
    }
}

/// A health component with current and maximum values.
/// The effect will preserve the current/max ratio when modifying max health.
#[derive(Component)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            current: 100.0,
            max: 100.0,
        }
    }
}

impl Health {
    pub fn new(current: f32, max: f32) -> Self {
        Self { current, max }
    }
}

// ============================================================================
// Effect types that modify components
// ============================================================================

/// Effect that modifies the Speed component.
#[derive(Event, Clone, Copy)]
pub struct SpeedModifier(pub ValueModifier);

impl StatusEffectApplicator<Speed> for SpeedModifier {
    fn modifier(&self) -> ValueModifier {
        self.0
    }

    fn apply(&self, component: &mut Speed, power: f32) {
        component.value = self.0.apply_scaled(component.value, power);
    }
}

/// Effect that modifies the Health component.
/// This example preserves the current/max health ratio when modifying max health.
#[derive(Event, Clone, Copy)]
pub struct MaxHealthModifier(pub ValueModifier);

impl StatusEffectApplicator<Health> for MaxHealthModifier {
    fn modifier(&self) -> ValueModifier {
        self.0
    }

    fn apply(&self, component: &mut Health, power: f32) {
        // Calculate the current health ratio
        let ratio = if component.max > 0.0 {
            component.current / component.max
        } else {
            1.0
        };

        // Apply the modifier to max health
        component.max = self.0.apply_scaled(component.max, power);

        // Preserve the health ratio
        component.current = component.max * ratio;
    }
}

// ============================================================================
// Plugin setup
// ============================================================================

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        // Register Speed effect with square root scaling (diminishing returns)
        .add_plugins(StatusEffectPlugin::<Speed, SpeedModifier>::new(
            StatusEffectApplication::sqrt(),
        ))
        // Register Health effect with linear scaling (no diminishing returns)
        .add_plugins(StatusEffectPlugin::<Health, MaxHealthModifier>::default())
        .add_systems(Startup, setup)
        .add_systems(Update, apply_effects_demo)
        .run();
}

/// Marker component to track our demo entity
#[derive(Component)]
struct Player;

/// Counter to track demo progression
#[derive(Resource, Default)]
struct DemoState {
    frame: u32,
}

fn setup(mut commands: Commands) {
    // Spawn a player entity with Speed and Health components
    commands.spawn((
        Player,
        Speed::new(100.0),
        Health::new(80.0, 100.0), // 80% health
    ));

    // Initialize demo state
    commands.insert_resource(DemoState::default());

    println!("=== msg_status_effect Example ===\n");
    println!("Initial state:");
    println!("  Speed: 100.0");
    println!("  Health: 80/100 (80%)\n");
}

fn apply_effects_demo(
    mut commands: Commands,
    query: Query<(Entity, &Speed, &Health), With<Player>>,
    mut state: ResMut<DemoState>,
) {
    state.frame += 1;

    let Ok((entity, speed, health)) = query.single() else {
        return;
    };

    match state.frame {
        // Frame 2: Apply a +50% speed buff
        2 => {
            println!("Frame {}: Applying +50% speed buff (sqrt scaling)", state.frame);
            commands.trigger(ApplyStatusEffect {
                effect: SpeedModifier(ValueModifier::Percent(50.0)),
                entity,
            });
        }
        // Frame 4: Apply another +50% speed buff (demonstrates diminishing returns)
        4 => {
            println!(
                "Frame {}: Current speed: {:.2}",
                state.frame, speed.value
            );
            println!("Frame {}: Applying another +50% speed buff", state.frame);
            commands.trigger(ApplyStatusEffect {
                effect: SpeedModifier(ValueModifier::Percent(50.0)),
                entity,
            });
        }
        // Frame 6: Show speed after second buff
        6 => {
            println!(
                "Frame {}: Speed after two +50% buffs: {:.2}",
                state.frame, speed.value
            );
            println!("  (With linear scaling it would be 225.0, but sqrt gives diminishing returns)\n");
        }
        // Frame 8: Apply +30 flat speed
        8 => {
            println!("Frame {}: Applying +30 flat speed (sqrt scaling)", state.frame);
            commands.trigger(ApplyStatusEffect {
                effect: SpeedModifier(ValueModifier::Val(30.0)),
                entity,
            });
        }
        // Frame 10: Show speed and apply health buff
        10 => {
            println!(
                "Frame {}: Speed after +30 flat: {:.2}",
                state.frame, speed.value
            );
            println!("  (Uses Pythagorean addition: sqrt(current^2 + 30^2))\n");

            println!("Frame {}: Applying +50 max health (linear scaling)", state.frame);
            commands.trigger(ApplyStatusEffect {
                effect: MaxHealthModifier(ValueModifier::Val(50.0)),
                entity,
            });
        }
        // Frame 12: Show health changes
        12 => {
            println!(
                "Frame {}: Health after +50 max: {:.1}/{:.1} (still {}%)",
                state.frame,
                health.current,
                health.max,
                ((health.current / health.max) * 100.0) as u32
            );
            println!("  (Health ratio preserved when max health changes)\n");
        }
        // Frame 14: Apply -20% speed debuff
        14 => {
            println!("Frame {}: Applying -20% speed debuff", state.frame);
            commands.trigger(ApplyStatusEffect {
                effect: SpeedModifier(ValueModifier::Percent(-20.0)),
                entity,
            });
        }
        // Frame 16: Show final state
        16 => {
            println!(
                "Frame {}: Final speed after -20% debuff: {:.2}",
                state.frame, speed.value
            );
            println!(
                "Frame {}: Final health: {:.1}/{:.1}\n",
                state.frame, health.current, health.max
            );
            println!("=== Demo Complete ===");
            std::process::exit(0);
        }
        _ => {}
    }
}
