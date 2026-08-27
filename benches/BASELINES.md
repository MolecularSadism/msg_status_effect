# Benchmark baselines

Criterion baseline `base`, captured 2026-08-26.

| | |
|---|---|
| Commit | `bceb2919feb5` |
| Branch | `claude/timed-status-module` |
| Toolchain | rustc 1.98.0 (88d9e12ae 2026-08-18) |
| Host | Linux x86_64 container (shared/virtualised) |

## Results

Mean with 95% confidence interval.

| Benchmark | Mean | 95% CI |
|---|---:|---|
| `next/fresh_activation` | 2.717 ns | 2.667 ns – 2.769 ns |
| `next/rejected_shorter` | 2.786 ns | 2.77 ns – 2.803 ns |
| `next/stack_up_overwrites` | 3.913 ns | 3.895 ns – 3.934 ns |
| `next/timer_over_milestone` | 3.341 ns | 3.245 ns – 3.452 ns |
| `overwritten_by/matrix` | 15.7 ns | 15.62 ns – 15.78 ns |

## Reproducing

The bench target must be named. A bare `cargo bench` also runs the lib test
harness, which rejects criterion's flags with
`error: Unrecognized option: 'baseline'`.

```sh
# capture
cargo bench --bench stacking -- --save-baseline base

# compare against it
cargo bench --bench stacking -- --baseline base
```

## How much to trust these

Taken in a shared virtualised container. Treat them as an order-of-magnitude
record, not a regression gate.

Re-running `take_batch/3x1000_pending` on byte-identical code about an hour
later on the *same* host reported `+31%` with `p = 0.00`. Criterion's
significance test measures sampling noise within a run; it cannot see the
host's load drifting between runs. So a reported change of this size here is
not evidence of a real regression.

To draw a conclusion from a comparison, capture the baseline and the
comparison back to back in one sitting, and treat anything under roughly
1.5x as inconclusive. Comparing these absolute numbers against a different
machine is meaningless.
