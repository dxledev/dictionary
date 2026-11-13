# Search and rendering optimization audit

Date: 2026-11-13

## Changes reviewed

- Similar-word ranking now retains a bounded candidate pool and sorts once at
  completion. Duplicate filtering and score-descending, lexical-ascending tie
  order are preserved within the retained pool. The scan remains O(pool size)
  per positive candidate; repeated sorting is removed.
- Daily-word home rendering borrows the existing `DailyWord` strings instead
  of constructing a temporary `Entry` with cloned strings and vectors.
- Theme polling checks the weak entity after every timer wake, including ticks
  that skip synchronization and synchronization failures.

## Performance evidence and open work

The search audit did not produce a reliable before/after benchmark. No numeric
speedup is claimed. Measure both revisions in release mode with an identical
profile and bundled word list before making a quantitative claim.

The modal backdrop still constructs 33 still-page samples. Any optimization must
preserve current layer weights, offsets, and blur, and needs isolated visual
validation. No appearance change was made in this pass.

## Validation

- `cargo test --offline`: 72 passed, 0 failed, 4 ignored.
- `python3 -m unittest discover -s scripts -p 'test_urban_pos.py'`: 8 passed.
- `cargo build --release --offline`: passed.
- Installation and installed-binary comparison: pending filesystem write
  permission for `/home/dxle/.local/bin`.
- No active application was launched; existing isolated GPUI tests ran as part
  of the Rust test suite.
