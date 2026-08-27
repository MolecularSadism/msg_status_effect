//! Timed status holds: components that stay on an entity while a release
//! condition is pending, with stacking rules, duration modifiers, and an
//! observable release seam.
//!
//! A *hold* is a presence-as-state component (e.g. a stun or a snare): the
//! entity carries it only while actually held, so hot queries exclude held
//! entities with `Without<C>` instead of fetching and branching. This module
//! provides the generic machinery around such components:
//!
//! - [`ReleaseCondition`] — when a hold lifts: never ([`Permanent`]), after a
//!   duration ([`Time`]), or on a host-defined ordered [`Milestone`] (an
//!   animation finishing, say — the milestone type and what reports it stay in
//!   the host).
//! - [`StatusHold`] — the hold's data: its release condition, an optional
//!   host-defined restore payload (the state to return to), and the runtime
//!   timer for timed holds.
//! - [`ReleaseCondition::overwritten_by`] / [`StatusHold::next`] — the
//!   stacking rules for how a re-application interacts with a running hold.
//! - [`TimedStatus`] — trait linking a hold component to its milestone and
//!   restore types; gives it [`tick_timed_status`] and [`release_hold`].
//! - [`TimerStatusEffect`] + [`DurationModifier`] — scale a running hold's
//!   timer (a resistance perk shortening stuns, say), registered through
//!   [`DurationModifierPlugin`] so unheld entities stay untouched.
//! - [`StatusReleased`] — entity event fired whenever a hold is released, so
//!   the host decides what release *does* (restore a movement state, replay an
//!   animation) instead of this module hardcoding it.
//!
//! What deliberately stays in the host: the concrete hold components, the
//! events/observers that apply them, and anything that interprets the restore
//! payload or reports milestones.
//!
//! [`Permanent`]: ReleaseCondition::Permanent
//! [`Time`]: ReleaseCondition::Time
//! [`Milestone`]: ReleaseCondition::Milestone

use std::marker::PhantomData;
use std::time::Duration;

use bevy::prelude::*;

use crate::{
    ApplyStatusEffect, MutableComponent, StatusEffectApplication, StatusEffectApplicator,
    ValueModifier,
};

// ============================================================================
// Release Conditions & Stacking
// ============================================================================

/// When a timed hold lifts.
///
/// `M` is the host's milestone type: an `Ord` enum whose ordering encodes
/// *strictness* (a stricter milestone is greater). Hosts without milestones
/// can use the default `()`.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub enum ReleaseCondition<M = ()> {
    /// Never lifts on its own; removed by hand or replaced by stacking.
    Permanent,
    /// Lifts once the duration has elapsed.
    Time(Duration),
    /// Lifts when the host reports a milestone at least as strict as this one
    /// (see [`StatusHold::released_by`]).
    Milestone(M),
}

impl<M: Ord> ReleaseCondition<M> {
    /// Whether an incoming condition should overwrite this (the existing)
    /// one when a status is re-applied.
    ///
    /// Stacking rules:
    /// - `Permanent` always wins (never overwritten; always overwrites)
    /// - `Time` vs `Time`: overwrite only if the new duration is strictly
    ///   longer — re-applying an *equal* duration deliberately never
    ///   refreshes a running hold
    /// - Milestone conditions don't overwrite timers or `Permanent`; timers
    ///   overwrite milestone conditions
    /// - Between milestones: the stricter (greater) one wins; equal doesn't
    ///   overwrite
    ///
    /// `Time` vs `Time` compares the two release conditions, so a hold whose
    /// duration changes while it runs (a [`DurationModifier`], say) must keep
    /// its `release` in sync with its timer — [`StatusHold::set_duration`]
    /// does exactly that.
    ///
    /// Returns true if `new` should overwrite this condition.
    #[must_use]
    pub fn overwritten_by(&self, new: &Self) -> bool {
        match (self, new) {
            // Permanent never gets overwritten
            (Self::Permanent, _) => false,

            // Overwrite with Permanent
            (_, Self::Permanent) => true,

            // Time vs Time: overwrite if new has longer duration
            (Self::Time(existing_dur), Self::Time(new_dur)) => new_dur > existing_dur,

            // Timer vs milestone: timer wins (don't overwrite timer with milestone)
            (Self::Time(_), _) => false,

            // Milestone vs timer: timer wins (overwrite milestone with timer)
            (_, Self::Time(_)) => true,

            // Milestone vs milestone: strictest condition wins
            (Self::Milestone(existing_m), Self::Milestone(new_m)) => new_m > existing_m,
        }
    }
}

// ============================================================================
// StatusHold
// ============================================================================

/// Data for an active timed hold: the condition that lifts it, the
/// host-defined state to restore on release, and (for timed holds) the runtime
/// countdown.
///
/// `M` is the host's milestone type, `S` its restore payload (the state the
/// entity was in before the hold — this module never interprets it, it only
/// carries it to [`StatusReleased`]).
#[derive(Reflect, Debug, Clone)]
pub struct StatusHold<M = (), S = ()> {
    /// The condition that lifts this hold.
    pub release: ReleaseCondition<M>,
    /// Host-defined state to restore when the hold lifts.
    pub previous_state: Option<S>,
    /// Runtime timer, created from a [`ReleaseCondition::Time`] duration on apply.
    pub timer: Option<Timer>,
}

// Manual Default impls so `M`/`S` need not implement Default themselves
// (a derive would add `M: Default`/`S: Default` bounds).
#[allow(clippy::derivable_impls)]
impl<M> Default for ReleaseCondition<M> {
    fn default() -> Self {
        Self::Permanent
    }
}

impl<M, S> Default for StatusHold<M, S> {
    fn default() -> Self {
        Self {
            release: ReleaseCondition::default(),
            previous_state: None,
            timer: None,
        }
    }
}

impl<M, S> StatusHold<M, S> {
    /// Rewrites this hold to lift after `duration`, keeping [`release`] and
    /// [`timer`] consistent: the release condition becomes
    /// [`ReleaseCondition::Time`] with the new duration and the countdown
    /// restarts from it.
    ///
    /// Use this instead of writing the two fields independently whenever a
    /// running hold's duration changes ([`DurationModifier`] does), so that
    /// stacking decisions — which compare release conditions, see
    /// [`ReleaseCondition::overwritten_by`] — judge against the live duration
    /// rather than the originally applied one.
    ///
    /// [`release`]: Self::release
    /// [`timer`]: Self::timer
    pub fn set_duration(&mut self, duration: Duration) {
        self.release = ReleaseCondition::Time(duration);
        self.timer = Some(Timer::new(duration, TimerMode::Once));
    }
}

impl<M: Ord, S> StatusHold<M, S> {
    /// Whether an occurred milestone satisfies this hold's release condition.
    ///
    /// True when the hold releases on a milestone at most as strict as the one
    /// that occurred — e.g. with milestones `Changed < CycleFinished <
    /// Finished`, a `Finished` report releases holds waiting on any of the
    /// three, while a `CycleFinished` report leaves `Finished` holds in place.
    #[must_use]
    pub fn released_by(&self, milestone: &M) -> bool {
        matches!(&self.release, ReleaseCondition::Milestone(required) if milestone >= required)
    }

    /// Builds the hold to write for an incoming status application, or `None`
    /// if an existing hold outranks the incoming one (see
    /// [`ReleaseCondition::overwritten_by`]) and nothing should change.
    ///
    /// `existing` is `None` when the entity is not currently held — with
    /// presence-as-state that is exactly "the component is absent", which is
    /// also what makes the application a fresh activation.
    ///
    /// The original `previous_state` is preserved across a stack-up; the
    /// `current_state` fallback is only used when there was no hold to inherit
    /// it from (otherwise a fresh read of the live state — which is the *held*
    /// state by then — would clobber the state to restore).
    #[must_use]
    pub fn next(
        existing: Option<&Self>,
        release: &ReleaseCondition<M>,
        current_state: Option<S>,
    ) -> Option<Self>
    where
        M: Clone,
        S: Clone,
    {
        if let Some(existing) = existing
            && !existing.release.overwritten_by(release)
        {
            return None;
        }

        let previous_state = existing
            .and_then(|h| h.previous_state.clone())
            .or(current_state);

        let timer = match release {
            ReleaseCondition::Time(dur) => Some(Timer::new(*dur, TimerMode::Once)),
            _ => None,
        };

        Some(Self {
            release: release.clone(),
            previous_state,
            timer,
        })
    }
}

// ============================================================================
// TimedStatus Trait & Systems
// ============================================================================

/// Trait linking a presence-as-state hold component to its [`StatusHold`].
///
/// Implementing this gives the component [`tick_timed_status`] (or the
/// [`TimedStatusPlugin`] wrapper) and [`release_hold`], with releases
/// announced via [`StatusReleased`].
pub trait TimedStatus: MutableComponent {
    /// The host's milestone type; `()` when unused.
    type Milestone: Ord + Send + Sync + 'static;
    /// The host's restore payload; `()` when unused.
    type Restore: Clone + Send + Sync + 'static;

    /// The hold data this component carries.
    fn hold(&self) -> &StatusHold<Self::Milestone, Self::Restore>;

    /// Mutable access to the hold data.
    fn hold_mut(&mut self) -> &mut StatusHold<Self::Milestone, Self::Restore>;
}

/// Entity event fired whenever a hold component `C` is released.
///
/// This is the release seam: the module removes the component and announces
/// the release; observers in the host decide what release *does* — restore
/// the carried `previous_state`, replay an animation, nothing at all.
#[derive(EntityEvent, Debug)]
pub struct StatusReleased<C: TimedStatus> {
    /// The entity whose hold was released.
    pub entity: Entity,
    /// The restore payload the hold carried.
    pub previous_state: Option<C::Restore>,
}

impl<C: TimedStatus> StatusReleased<C> {
    /// Creates a release announcement for `entity`.
    #[must_use]
    pub fn new(entity: Entity, previous_state: Option<C::Restore>) -> Self {
        Self {
            entity,
            previous_state,
        }
    }
}

/// Drops the hold component and announces the release via [`StatusReleased`].
///
/// Callable from any host system that decides a hold should lift (a milestone
/// report, a cleanse effect); the timer path calls it from
/// [`tick_timed_status`].
///
/// Removal and announcement happen together when the queued command applies:
/// [`StatusReleased`] fires only if the component was actually still present,
/// so release paths racing within one frame (a timer expiry plus a host
/// cleanse, say) announce a single release.
pub fn release_hold<C: TimedStatus>(
    commands: &mut Commands,
    entity: Entity,
    previous_state: Option<C::Restore>,
) {
    commands.queue(move |world: &mut World| {
        let taken = world
            .get_entity_mut(entity)
            .ok()
            .and_then(|mut entity_mut| entity_mut.take::<C>());
        if taken.is_some() {
            world.trigger(StatusReleased::<C>::new(entity, previous_state));
        }
    });
}

/// Ticks the timer of every active hold `C` and releases finished ones.
///
/// Only actually-held entities carry the component, so this iterates the held
/// set rather than every entity. Holds without a timer (permanent or
/// milestone-released) are detected through an immutable read and skipped, so
/// they are never spuriously marked `Changed<C>`; timed holds are necessarily
/// marked changed each tick as their countdown advances.
pub fn tick_timed_status<C: TimedStatus>(
    mut commands: Commands,
    time: Res<Time>,
    mut q_held: Query<(Entity, &mut C)>,
) {
    for (entity, mut held) in q_held.iter_mut() {
        if held.hold().timer.is_none() {
            continue;
        }
        let Some(timer) = held.hold_mut().timer.as_mut() else {
            continue;
        };
        timer.tick(time.delta());
        if timer.just_finished() {
            let previous_state = held.hold().previous_state.clone();
            release_hold::<C>(&mut commands, entity, previous_state);
        }
    }
}

/// Registers [`tick_timed_status`] for hold component `C` in `FixedUpdate`.
///
/// Add one per hold component. Hosts that need a different schedule or system
/// set add [`tick_timed_status`] themselves instead.
pub struct TimedStatusPlugin<C: TimedStatus> {
    _marker: PhantomData<C>,
}

impl<C: TimedStatus> Default for TimedStatusPlugin<C> {
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<C: TimedStatus> Plugin for TimedStatusPlugin<C> {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, tick_timed_status::<C>);
    }
}

// ============================================================================
// Timer Status Effect Trait & Generic Duration Modifier
// ============================================================================

/// Trait for components that hold a scalable runtime [`Timer`].
///
/// Implementing this gives you a [`DurationModifier<C>`] event for free,
/// registered through [`DurationModifierPlugin`], that scales the timer's
/// remaining duration. [`TimedStatus`] implementors satisfy it by returning
/// `self.hold_mut().timer.as_mut()` from [`timer_mut`] and forwarding
/// [`set_duration`] to [`StatusHold::set_duration`], which keeps the hold's
/// release condition in sync with the rewritten timer.
///
/// [`timer_mut`]: Self::timer_mut
/// [`set_duration`]: Self::set_duration
pub trait TimerStatusEffect: MutableComponent {
    /// Returns a mutable reference to the runtime timer (if present).
    fn timer_mut(&mut self) -> Option<&mut Timer>;

    /// Replaces the runtime countdown so `duration` remains, updating any
    /// bookkeeping that mirrors the timer (a [`StatusHold`]'s release
    /// condition, say) so later stacking decisions see the changed duration.
    fn set_duration(&mut self, duration: Duration);
}

/// Generic duration modifier that scales the [`Timer`] inside any
/// [`TimerStatusEffect`] component.
///
/// Reusable for any hold with a runtime countdown. The modifier is applied to
/// the timer's *remaining* duration using the power parameter for scaling
/// based on perk level or item count. Register it with
/// [`DurationModifierPlugin`].
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use bevy::prelude::*;
/// use msg_status_effect::prelude::*;
///
/// #[derive(Component)]
/// struct Rooted(StatusHold);
///
/// impl TimerStatusEffect for Rooted {
///     fn timer_mut(&mut self) -> Option<&mut Timer> {
///         self.0.timer.as_mut()
///     }
///
///     fn set_duration(&mut self, duration: Duration) {
///         self.0.set_duration(duration);
///     }
/// }
///
/// fn plugin(app: &mut App) {
///     app.add_plugins(DurationModifierPlugin::<Rooted>::default());
/// }
///
/// // Shorten a running root by 20%:
/// fn apply_resistance(mut commands: Commands, entity: Entity) {
///     commands.trigger(ApplyStatusEffect {
///         entity,
///         effect: DurationModifier::<Rooted>::new(ValueModifier::Percent(-20.0)),
///     });
/// }
/// ```
#[derive(Event)]
pub struct DurationModifier<C: TimerStatusEffect> {
    /// How to change the timer's remaining duration.
    pub modifier: ValueModifier,
    _marker: PhantomData<C>,
}

// Manual impls so `C` need not implement Debug/Clone/Copy itself (a derive
// would add those bounds through the PhantomData field).
impl<C: TimerStatusEffect> std::fmt::Debug for DurationModifier<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurationModifier")
            .field("modifier", &self.modifier)
            .finish()
    }
}

impl<C: TimerStatusEffect> Clone for DurationModifier<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: TimerStatusEffect> Copy for DurationModifier<C> {}

impl<C: TimerStatusEffect> DurationModifier<C> {
    /// Creates a new `DurationModifier` with the given [`ValueModifier`].
    #[must_use]
    pub fn new(modifier: ValueModifier) -> Self {
        Self {
            modifier,
            _marker: PhantomData,
        }
    }
}

impl<C: TimerStatusEffect> StatusEffectApplicator<C> for DurationModifier<C> {
    fn modifier(&self) -> ValueModifier {
        self.modifier
    }

    fn apply(&self, component: &mut C, power: f32) {
        let Some(timer) = component.timer_mut() else {
            return;
        };
        let current_ms = timer.remaining().as_millis() as f32;
        let new_ms = self
            .modifier
            .apply_scaled(current_ms, power)
            .max(0.0)
            .round();
        component.set_duration(Duration::from_millis(new_ms as u64));
    }
}

/// Registers [`DurationModifier<C>`] handling for a [`TimerStatusEffect`]
/// component, with the given [`StatusEffectApplication`] power scaling.
///
/// Unlike [`StatusEffectPlugin`], the registered observer never inserts `C`:
/// a duration modifier only means anything against a hold that is already
/// running, and under presence-as-state semantics inserting a default hold
/// would *hold* the entity. Targeting an entity that does not carry `C` is
/// therefore a no-op.
///
/// [`StatusEffectPlugin`]: crate::StatusEffectPlugin
pub struct DurationModifierPlugin<C: TimerStatusEffect> {
    config: StatusEffectApplication<C>,
}

impl<C: TimerStatusEffect> Default for DurationModifierPlugin<C> {
    fn default() -> Self {
        Self {
            config: StatusEffectApplication::default(),
        }
    }
}

impl<C: TimerStatusEffect> DurationModifierPlugin<C> {
    /// Creates the plugin with the specified scaling configuration.
    #[must_use]
    pub fn new(config: StatusEffectApplication<C>) -> Self {
        Self { config }
    }
}

impl<C: TimerStatusEffect> Plugin for DurationModifierPlugin<C> {
    fn build(&self, app: &mut App) {
        app.insert_resource(StatusEffectApplication::<C>::with_power(self.config.power));
        app.add_observer(apply_duration_modifier::<C>);
    }
}

/// Observer applying [`DurationModifier<C>`] strictly to entities that
/// currently carry `C`; unheld entities are left untouched (see
/// [`DurationModifierPlugin`]).
fn apply_duration_modifier<C: TimerStatusEffect>(
    on: On<ApplyStatusEffect<DurationModifier<C>>>,
    config: Res<StatusEffectApplication<C>>,
    mut q: Query<&mut C>,
) {
    if let Ok(mut component) = q.get_mut(on.entity) {
        on.effect.apply(&mut component, config.power);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApplyStatusEffect;
    use bevy::ecs::system::RunSystemOnce;

    /// Dummy milestone type: ordering encodes strictness, mirroring an
    /// animation-event ladder.
    #[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestMilestone {
        Changed,
        CycleFinished,
        Finished,
    }

    /// Dummy restore payload, standing in for a host's movement state.
    #[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
    enum TestRestore {
        JumpAndRun,
        Fly,
    }

    type TestHold = StatusHold<TestMilestone, TestRestore>;
    type TestCondition = ReleaseCondition<TestMilestone>;

    /// Dummy hold component, standing in for a host's stun/snare.
    /// Deliberately neither `Default` nor `Clone`: the machinery must not
    /// require either.
    #[derive(Component)]
    struct Held(TestHold);

    impl TimedStatus for Held {
        type Milestone = TestMilestone;
        type Restore = TestRestore;

        fn hold(&self) -> &TestHold {
            &self.0
        }

        fn hold_mut(&mut self) -> &mut TestHold {
            &mut self.0
        }
    }

    impl TimerStatusEffect for Held {
        fn timer_mut(&mut self) -> Option<&mut Timer> {
            self.0.timer.as_mut()
        }

        fn set_duration(&mut self, duration: Duration) {
            self.0.set_duration(duration);
        }
    }

    // -------------------------------------------------------------------
    // ReleaseCondition::overwritten_by — stacking comparator
    // -------------------------------------------------------------------

    #[test]
    fn overwritten_by_permanent_never_overwritten() {
        let permanent = TestCondition::Permanent;
        let timed = TestCondition::Time(Duration::from_secs(5));
        let cycle = TestCondition::Milestone(TestMilestone::CycleFinished);
        let finished = TestCondition::Milestone(TestMilestone::Finished);

        assert!(!permanent.overwritten_by(&timed));
        assert!(!permanent.overwritten_by(&cycle));
        assert!(!permanent.overwritten_by(&finished));
        assert!(!permanent.overwritten_by(&permanent));
    }

    #[test]
    fn overwritten_by_permanent_always_wins() {
        let permanent = TestCondition::Permanent;
        let timed = TestCondition::Time(Duration::from_secs(5));
        let cycle = TestCondition::Milestone(TestMilestone::CycleFinished);
        let finished = TestCondition::Milestone(TestMilestone::Finished);

        assert!(timed.overwritten_by(&permanent));
        assert!(cycle.overwritten_by(&permanent));
        assert!(finished.overwritten_by(&permanent));
    }

    #[test]
    fn overwritten_by_timer_duration_comparison() {
        let short = TestCondition::Time(Duration::from_secs(2));
        let long = TestCondition::Time(Duration::from_secs(5));

        // Longer duration should overwrite shorter
        assert!(short.overwritten_by(&long));
        // Shorter duration should not overwrite longer
        assert!(!long.overwritten_by(&short));
        // Equal duration deliberately never refreshes
        assert!(!short.overwritten_by(&short));
    }

    #[test]
    fn overwritten_by_timer_beats_milestone() {
        let timed = TestCondition::Time(Duration::from_secs(1));
        let changed = TestCondition::Milestone(TestMilestone::Changed);
        let cycle = TestCondition::Milestone(TestMilestone::CycleFinished);
        let finished = TestCondition::Milestone(TestMilestone::Finished);

        // Timer should overwrite milestone conditions
        assert!(changed.overwritten_by(&timed));
        assert!(cycle.overwritten_by(&timed));
        assert!(finished.overwritten_by(&timed));

        // Milestone conditions should not overwrite timer
        assert!(!timed.overwritten_by(&changed));
        assert!(!timed.overwritten_by(&cycle));
        assert!(!timed.overwritten_by(&finished));
    }

    #[test]
    fn overwritten_by_milestone_strictness_order() {
        let changed = TestCondition::Milestone(TestMilestone::Changed);
        let cycle = TestCondition::Milestone(TestMilestone::CycleFinished);
        let finished = TestCondition::Milestone(TestMilestone::Finished);

        // Finished (strictest) beats both others
        assert!(cycle.overwritten_by(&finished));
        assert!(changed.overwritten_by(&finished));
        assert!(!finished.overwritten_by(&cycle));
        assert!(!finished.overwritten_by(&changed));

        // CycleFinished beats Changed
        assert!(changed.overwritten_by(&cycle));
        assert!(!cycle.overwritten_by(&changed));
    }

    #[test]
    fn overwritten_by_same_milestone_condition() {
        for milestone in [
            TestMilestone::Changed,
            TestMilestone::CycleFinished,
            TestMilestone::Finished,
        ] {
            let condition = TestCondition::Milestone(milestone);
            assert!(!condition.overwritten_by(&condition));
        }
    }

    #[test]
    fn release_condition_equality() {
        assert_eq!(
            TestCondition::Time(Duration::from_millis(500)),
            TestCondition::Time(Duration::from_millis(500))
        );
        assert_ne!(
            TestCondition::Time(Duration::from_millis(500)),
            TestCondition::Time(Duration::from_millis(1000))
        );
        assert_ne!(
            TestCondition::Permanent,
            TestCondition::Milestone(TestMilestone::CycleFinished)
        );
    }

    // -------------------------------------------------------------------
    // released_by — milestone strictness at release time
    // -------------------------------------------------------------------

    #[test]
    fn released_by_accepts_equal_or_stricter_milestones() {
        let hold = TestHold {
            release: TestCondition::Milestone(TestMilestone::CycleFinished),
            ..Default::default()
        };
        assert!(hold.released_by(&TestMilestone::Finished));
        assert!(hold.released_by(&TestMilestone::CycleFinished));
        assert!(!hold.released_by(&TestMilestone::Changed));

        let timed = TestHold {
            release: TestCondition::Time(Duration::from_secs(1)),
            ..Default::default()
        };
        assert!(!timed.released_by(&TestMilestone::Finished));
    }

    // -------------------------------------------------------------------
    // StatusHold::next — stacking rules against the presence-as-state shape
    // -------------------------------------------------------------------

    #[test]
    fn next_activates_when_absent() {
        let hold = TestHold::next(
            None,
            &TestCondition::Time(Duration::from_secs(2)),
            Some(TestRestore::JumpAndRun),
        )
        .expect("no existing hold means the application always lands");
        assert_eq!(hold.previous_state, Some(TestRestore::JumpAndRun));
        assert!(hold.timer.is_some());
    }

    #[test]
    fn next_stacks_longer_duration_without_losing_previous_state() {
        let existing = TestHold {
            release: TestCondition::Time(Duration::from_secs(1)),
            previous_state: Some(TestRestore::Fly),
            timer: Some(Timer::new(Duration::from_secs(1), TimerMode::Once)),
        };
        // A fresh read of the live state (which is the held state by now)
        // must not clobber the original previous_state.
        let hold = TestHold::next(
            Some(&existing),
            &TestCondition::Time(Duration::from_secs(5)),
            Some(TestRestore::JumpAndRun),
        )
        .expect("a longer duration overwrites");
        assert_eq!(
            hold.previous_state,
            Some(TestRestore::Fly),
            "the original pre-hold state is preserved across a stack-up"
        );
        assert_eq!(hold.release, TestCondition::Time(Duration::from_secs(5)));
    }

    #[test]
    fn next_skips_shorter_duration() {
        let existing = TestHold {
            release: TestCondition::Time(Duration::from_secs(5)),
            previous_state: None,
            timer: Some(Timer::new(Duration::from_secs(5), TimerMode::Once)),
        };
        assert!(
            TestHold::next(
                Some(&existing),
                &TestCondition::Time(Duration::from_secs(1)),
                None,
            )
            .is_none(),
            "a shorter duration does not overwrite"
        );
    }

    // -------------------------------------------------------------------
    // tick_timed_status / release_hold — presence is the state, releases
    // are observable
    // -------------------------------------------------------------------

    /// Collects the payload of every observed `StatusReleased<Held>`.
    #[derive(Resource, Default)]
    struct Releases(Vec<(Entity, Option<TestRestore>)>);

    fn observe_releases(on: On<StatusReleased<Held>>, mut releases: ResMut<Releases>) {
        releases.0.push((on.entity, on.previous_state));
    }

    fn tick_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Releases>();
        app.add_observer(observe_releases);
        app
    }

    #[test]
    fn hold_is_absent_until_applied_and_removed_on_release() {
        let mut app = tick_app();

        let entity = app.world_mut().spawn_empty().id();
        assert!(
            app.world().get::<Held>(entity).is_none(),
            "an unheld entity carries no component, so Without<Held> excludes nothing"
        );

        let hold = TestHold::next(
            None,
            &TestCondition::Time(Duration::from_secs(1)),
            Some(TestRestore::JumpAndRun),
        )
        .unwrap();
        app.world_mut().entity_mut(entity).insert(Held(hold));
        assert!(
            app.world().get::<Held>(entity).is_some(),
            "the hold is present while held"
        );

        // Tick the timer past its duration and let it release.
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs(2));
        app.world_mut()
            .run_system_once(tick_timed_status::<Held>)
            .unwrap();
        app.update(); // flush the release's remove + StatusReleased commands

        assert!(
            app.world().get::<Held>(entity).is_none(),
            "releasing drops the component so queries see the entity again"
        );
        assert_eq!(
            app.world().resource::<Releases>().0,
            vec![(entity, Some(TestRestore::JumpAndRun))],
            "the release seam reports the entity and its restore payload"
        );
    }

    #[test]
    fn permanent_and_milestone_holds_never_time_out() {
        let mut app = tick_app();

        let permanent = app
            .world_mut()
            .spawn(Held(TestHold {
                release: TestCondition::Permanent,
                ..Default::default()
            }))
            .id();
        let milestone = app
            .world_mut()
            .spawn(Held(TestHold {
                release: TestCondition::Milestone(TestMilestone::Finished),
                ..Default::default()
            }))
            .id();

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs(100));
        app.world_mut()
            .run_system_once(tick_timed_status::<Held>)
            .unwrap();
        app.update();

        assert!(app.world().get::<Held>(permanent).is_some());
        assert!(app.world().get::<Held>(milestone).is_some());
        assert!(app.world().resource::<Releases>().0.is_empty());
    }

    #[test]
    fn tick_leaves_untimed_holds_unmarked() {
        let mut app = tick_app();

        let permanent = app
            .world_mut()
            .spawn(Held(TestHold {
                release: TestCondition::Permanent,
                ..Default::default()
            }))
            .id();
        let milestone = app
            .world_mut()
            .spawn(Held(TestHold {
                release: TestCondition::Milestone(TestMilestone::Finished),
                ..Default::default()
            }))
            .id();
        let timed = app
            .world_mut()
            .spawn(Held(TestHold {
                release: TestCondition::Time(Duration::from_secs(10)),
                previous_state: None,
                timer: Some(Timer::new(Duration::from_secs(10), TimerMode::Once)),
            }))
            .id();

        app.update();
        app.world_mut().clear_trackers();

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(100));
        app.world_mut()
            .run_system_once(tick_timed_status::<Held>)
            .unwrap();

        let changed: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, Changed<Held>>()
            .iter(app.world())
            .collect();
        assert_eq!(
            changed,
            vec![timed],
            "only the ticking timed hold is marked changed, \
             not the permanent ({permanent}) or milestone ({milestone}) holds"
        );
    }

    #[test]
    fn release_hold_announces_from_host_systems_too() {
        let mut app = tick_app();
        let entity = app
            .world_mut()
            .spawn(Held(TestHold {
                release: TestCondition::Milestone(TestMilestone::Finished),
                previous_state: Some(TestRestore::Fly),
                timer: None,
            }))
            .id();

        // A host milestone system decides the hold lifts.
        app.world_mut()
            .run_system_once(move |mut commands: Commands, q: Query<&Held>| {
                let held = q.get(entity).unwrap();
                if held.hold().released_by(&TestMilestone::Finished) {
                    let previous = held.hold().previous_state;
                    release_hold::<Held>(&mut commands, entity, previous);
                }
            })
            .unwrap();
        app.update();

        assert!(app.world().get::<Held>(entity).is_none());
        assert_eq!(
            app.world().resource::<Releases>().0,
            vec![(entity, Some(TestRestore::Fly))]
        );
    }

    #[test]
    fn racing_releases_announce_once() {
        let mut app = tick_app();
        let entity = app
            .world_mut()
            .spawn(Held(TestHold {
                release: TestCondition::Milestone(TestMilestone::Finished),
                previous_state: Some(TestRestore::Fly),
                timer: None,
            }))
            .id();

        // Two release paths race in the same frame: both see the still-present
        // component and both queue a release.
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                release_hold::<Held>(&mut commands, entity, Some(TestRestore::Fly));
                release_hold::<Held>(&mut commands, entity, Some(TestRestore::Fly));
            })
            .unwrap();
        app.update();

        assert!(app.world().get::<Held>(entity).is_none());
        assert_eq!(
            app.world().resource::<Releases>().0,
            vec![(entity, Some(TestRestore::Fly))],
            "only the release that actually removed the component announces"
        );
    }

    #[test]
    fn timed_status_plugin_releases_through_fixed_update() {
        use bevy::time::TimeUpdateStrategy;

        let mut app = tick_app();
        app.add_plugins(TimedStatusPlugin::<Held>::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            50,
        )));

        let entity = app
            .world_mut()
            .spawn(Held(
                TestHold::next(
                    None,
                    &TestCondition::Time(Duration::from_millis(120)),
                    Some(TestRestore::JumpAndRun),
                )
                .unwrap(),
            ))
            .id();
        assert!(app.world().get::<Held>(entity).is_some());

        // Each update advances virtual time by 50ms; well past the 120ms hold
        // after a handful of frames' worth of FixedUpdate runs.
        for _ in 0..10 {
            app.update();
        }

        assert!(
            app.world().get::<Held>(entity).is_none(),
            "the plugin's FixedUpdate registration drives the tick to release"
        );
        assert_eq!(
            app.world().resource::<Releases>().0,
            vec![(entity, Some(TestRestore::JumpAndRun))]
        );
    }

    // -------------------------------------------------------------------
    // DurationModifier through the StatusEffectApplicator machinery
    // -------------------------------------------------------------------

    #[test]
    fn duration_modifier_scales_remaining_time() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(DurationModifierPlugin::<Held>::default());

        let entity = app
            .world_mut()
            .spawn(Held(
                TestHold::next(None, &TestCondition::Time(Duration::from_secs(10)), None).unwrap(),
            ))
            .id();

        // A resistance effect shortens the running hold by 50%.
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.trigger(ApplyStatusEffect {
                    entity,
                    effect: DurationModifier::<Held>::new(ValueModifier::Percent(-50.0)),
                });
            })
            .unwrap();
        app.update();

        let mut held = app.world_mut().get_mut::<Held>(entity).unwrap();
        let remaining = held.timer_mut().unwrap().remaining();
        assert!(
            (remaining.as_secs_f32() - 5.0).abs() < 0.01,
            "expected ~5s remaining, got {remaining:?}"
        );
    }

    #[test]
    fn duration_modifier_ignores_unheld_entities() {
        let mut app = tick_app();
        app.add_plugins(DurationModifierPlugin::<Held>::default());

        let entity = app.world_mut().spawn_empty().id();
        app.update();

        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.trigger(ApplyStatusEffect {
                    entity,
                    effect: DurationModifier::<Held>::new(ValueModifier::Percent(-50.0)),
                });
            })
            .unwrap();
        app.update();
        app.update();

        assert!(
            app.world().get::<Held>(entity).is_none(),
            "a duration modifier must not insert a default (permanent) hold"
        );
        assert!(
            app.world().resource::<Releases>().0.is_empty(),
            "nothing was held, so nothing releases"
        );
    }

    #[test]
    fn duration_modifier_keeps_release_condition_in_sync_for_stacking() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(DurationModifierPlugin::<Held>::default());

        let entity = app
            .world_mut()
            .spawn(Held(
                TestHold::next(None, &TestCondition::Time(Duration::from_secs(10)), None).unwrap(),
            ))
            .id();

        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.trigger(ApplyStatusEffect {
                    entity,
                    effect: DurationModifier::<Held>::new(ValueModifier::Percent(-50.0)),
                });
            })
            .unwrap();
        app.update();

        let held = app.world().get::<Held>(entity).unwrap();
        assert_eq!(
            held.hold().release,
            TestCondition::Time(Duration::from_secs(5)),
            "the release condition tracks the modified timer"
        );

        // Stacking judges against the modified duration, not the nominal 10s:
        // a 6s re-application now lands, a 4s one still doesn't.
        assert!(
            TestHold::next(
                Some(held.hold()),
                &TestCondition::Time(Duration::from_secs(6)),
                None,
            )
            .is_some()
        );
        assert!(
            TestHold::next(
                Some(held.hold()),
                &TestCondition::Time(Duration::from_secs(4)),
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn duration_modifier_never_goes_negative() {
        let mut held = Held(TestHold {
            release: TestCondition::Time(Duration::from_secs(1)),
            previous_state: None,
            timer: Some(Timer::new(Duration::from_secs(1), TimerMode::Once)),
        });
        DurationModifier::<Held>::new(ValueModifier::Val(-5000.0)).apply(&mut held, 1.0);
        assert_eq!(held.timer_mut().unwrap().remaining(), Duration::ZERO);
    }
}
