# 0001 — No unsafe Rust, permanently

Decided 2026-08-14. Recorded 2026-08-15. Status: standing.

## The decision

This crate does not use `unsafe`, and will not. The guarantee is compiler
enforced — `unsafe_code = "forbid"` under `[lints.rust]` in Cargo.toml — and
this record is the reasoning behind it, written down so the question does not
reopen every time read throughput comes up. It was not decided from taste. It
was decided from a measurement, a mechanism, two soundness failures, and a
safe-Rust result that made the whole argument moot. Those four legs, from the
campaign record:

1. **Measured, 2026-08-14.** With the forbid lifted and the unsafe candidates
   hand-written, 1 of 6 benchmark rows got better, 1 got worse — bitpacker
   write fell to 0.71x — and 4 were flat. The read path gained exactly zero,
   because LLVM already elides those checks.

2. **The regression's mechanism.** The safe check folds to a near-free
   `ubfx`+`cbnz`, and removing it perturbed register allocation into 10 extra
   128-bit stack copies per group. The check was not a cost the allocator was
   paying; it was a structure the allocator was using.

3. **Both prior unsafe blocks were UNSOUND** — each breaks the invariant
   `bits_written == word_index * 64 + scratch_bits` via `flush_bits` — and
   both carried confident SAFETY comments. Found by an adversarial verifier,
   with a reproducer.

4. **Decisive.** The read throughput unsafe was meant to buy came from SAFE
   Rust — a `#[cold]` error constructor plus seven `#[inline(always)]` — with
   `unsafe_code = "forbid"` untouched.

## The falsifier

A future measured read-path win that safe Rust demonstrably cannot reach
reopens the decision. Nothing weaker does: not an intuition about bounds
checks, not a benchmark from another crate, not a profile without a serious
safe-Rust attempt beside it.
