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

```sh
cargo bench -- --save-baseline base   # capture
cargo bench -- --baseline base        # compare against it
```

These were taken in a shared virtualised container, so absolute figures carry
more run-to-run noise than a dedicated machine. Comparisons made with
`--baseline base` on the same host are meaningful; comparing these absolute
numbers against a different machine is not.
