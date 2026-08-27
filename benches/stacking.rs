//! Benchmarks for the hold-stacking hot path: the
//! [`ReleaseCondition::overwritten_by`] comparator and [`StatusHold::next`],
//! which run on every status re-application.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use msg_status_effect::prelude::{ReleaseCondition, StatusHold};

/// Host-style milestone ladder: ordering encodes strictness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Milestone {
    Changed,
    CycleFinished,
    Finished,
}

/// Host-style restore payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Restore {
    JumpAndRun,
    Fly,
}

type Condition = ReleaseCondition<Milestone>;
type Hold = StatusHold<Milestone, Restore>;

/// The full comparator matrix: every variant pairing appears at least once.
fn condition_pairs() -> Vec<(Condition, Condition)> {
    let permanent = Condition::Permanent;
    let short = Condition::Time(Duration::from_secs(2));
    let long = Condition::Time(Duration::from_secs(5));
    let changed = Condition::Milestone(Milestone::Changed);
    let cycle = Condition::Milestone(Milestone::CycleFinished);
    let finished = Condition::Milestone(Milestone::Finished);

    vec![
        (permanent.clone(), long.clone()),
        (permanent.clone(), finished.clone()),
        (short.clone(), permanent.clone()),
        (short.clone(), long.clone()),
        (long.clone(), short.clone()),
        (short.clone(), short.clone()),
        (long, cycle.clone()),
        (changed.clone(), short),
        (cycle.clone(), finished.clone()),
        (finished, changed),
        (cycle.clone(), cycle),
        (Condition::Permanent, permanent),
    ]
}

fn bench_overwritten_by(c: &mut Criterion) {
    let pairs = condition_pairs();
    c.bench_function("overwritten_by/matrix", |b| {
        b.iter(|| {
            let mut overwrites = 0usize;
            for (existing, new) in &pairs {
                if black_box(existing).overwritten_by(black_box(new)) {
                    overwrites += 1;
                }
            }
            overwrites
        });
    });
}

fn bench_next(c: &mut Criterion) {
    let running = Hold {
        release: Condition::Time(Duration::from_secs(5)),
        previous_state: Some(Restore::Fly),
        timer: None,
    };
    let milestone_hold = Hold {
        release: Condition::Milestone(Milestone::CycleFinished),
        previous_state: Some(Restore::JumpAndRun),
        timer: None,
    };
    let longer = Condition::Time(Duration::from_secs(8));
    let shorter = Condition::Time(Duration::from_secs(2));

    c.bench_function("next/fresh_activation", |b| {
        b.iter(|| {
            Hold::next(
                black_box(None),
                black_box(&longer),
                black_box(Some(Restore::JumpAndRun)),
            )
        });
    });

    c.bench_function("next/stack_up_overwrites", |b| {
        b.iter(|| {
            Hold::next(
                black_box(Some(&running)),
                black_box(&longer),
                black_box(Some(Restore::JumpAndRun)),
            )
        });
    });

    c.bench_function("next/rejected_shorter", |b| {
        b.iter(|| {
            Hold::next(
                black_box(Some(&running)),
                black_box(&shorter),
                black_box(None),
            )
        });
    });

    c.bench_function("next/timer_over_milestone", |b| {
        b.iter(|| {
            Hold::next(
                black_box(Some(&milestone_hold)),
                black_box(&shorter),
                black_box(Some(Restore::Fly)),
            )
        });
    });
}

criterion_group!(benches, bench_overwritten_by, bench_next);
criterion_main!(benches);
