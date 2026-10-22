//! The negative controls: a reader with ONE refusal rule deleted.
//!
//! STANDARD.md, "A gate nobody has seen go red is a claim, not a measurement": for each
//! refusal rule, a reader with that one rule deleted, run against the real corpus, must fail
//! exactly the vectors its `conformance-controls/<rule>.expected` file names and no others.
//! `tests/conformance.rs` runs every control below against the vendored corpus and compares
//! the failing set with the vendored `.expected` list.
//!
//! The switch exists only in a build configured with `--cfg serialize_controls` (CI sets it
//! through `RUSTFLAGS` for one test step). Every other build compiles [`deleted`] to the
//! constant `false`, so each guarded rule is exactly the unguarded rule and the read path
//! carries nothing of this module. The active control is thread local, so the corpus can run
//! sabotaged on one test thread while the real corpus runs on another.

/// One refusal rule a control deletes, named as its `conformance-controls/<rule>.expected`
/// file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// `align-padding`: readers verify that the alignment padding bits are zero.
    AlignPadding,
    /// `ranged-reconstruction`: a ranged int decodes to its offset plus min.
    RangedReconstruction,
    /// `ranged-range`: a ranged int whose offset is past the span is refused.
    RangedRange,
    /// `fixed-degenerate-terminal`: every read consults the failure state, the zero bit
    /// degenerate fixed point read included.
    FixedDegenerateTerminal,
    /// `float-finite`: a float read refuses NaN and the infinities.
    FloatFinite,
    /// `double-finite`: a double read refuses NaN and the infinities.
    DoubleFinite,
    /// `compressed-float-finite`: a compressed float decode that is not finite is refused.
    CompressedFloatFinite,
    /// `end-trailing-data`: an end refuses a whole byte after the object.
    EndTrailingData,
    /// `end-padding`: an end refuses non-zero padding in the final byte.
    EndPadding,
    /// `int-relative-canonical`: the absolute tier refuses a difference a bounded tier
    /// carries.
    IntRelativeCanonical,
    /// `string-embedded-nul`: a string read refuses a zero byte among its payload.
    StringEmbeddedNul,
    /// `string-utf8`: a string read refuses a payload that is not well-formed UTF-8.
    StringUtf8,
    /// `string-utf8-surrogate`: a string read refuses an encoded surrogate code point (one
    /// clause of the UTF-8 rule alone).
    StringUtf8Surrogate,
    /// `int64-through-double`: a 64 bit value is carried exactly, never through a double.
    Int64ThroughDouble,
    /// `wrong-reason`: a refusal names the rule that refused it (this control reports
    /// `invalid_utf8` where `embedded_nul` is due).
    WrongReason,
}

#[cfg(serialize_controls)]
impl Control {
    /// Every control, in the reference's order.
    pub const ALL: [Control; 15] = [
        Control::AlignPadding,
        Control::RangedReconstruction,
        Control::RangedRange,
        Control::FixedDegenerateTerminal,
        Control::FloatFinite,
        Control::DoubleFinite,
        Control::CompressedFloatFinite,
        Control::EndTrailingData,
        Control::EndPadding,
        Control::IntRelativeCanonical,
        Control::StringEmbeddedNul,
        Control::StringUtf8,
        Control::StringUtf8Surrogate,
        Control::Int64ThroughDouble,
        Control::WrongReason,
    ];

    /// The control's name, which is its `.expected` file's stem.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Control::AlignPadding => "align-padding",
            Control::RangedReconstruction => "ranged-reconstruction",
            Control::RangedRange => "ranged-range",
            Control::FixedDegenerateTerminal => "fixed-degenerate-terminal",
            Control::FloatFinite => "float-finite",
            Control::DoubleFinite => "double-finite",
            Control::CompressedFloatFinite => "compressed-float-finite",
            Control::EndTrailingData => "end-trailing-data",
            Control::EndPadding => "end-padding",
            Control::IntRelativeCanonical => "int-relative-canonical",
            Control::StringEmbeddedNul => "string-embedded-nul",
            Control::StringUtf8 => "string-utf8",
            Control::StringUtf8Surrogate => "string-utf8-surrogate",
            Control::Int64ThroughDouble => "int64-through-double",
            Control::WrongReason => "wrong-reason",
        }
    }
}

#[cfg(serialize_controls)]
std::thread_local! {
    static ACTIVE: core::cell::Cell<Option<Control>> = const { core::cell::Cell::new(None) };
}

/// Delete one rule on the calling thread (`None` restores the real reader).
#[cfg(serialize_controls)]
pub fn set_control(control: Option<Control>) {
    ACTIVE.with(|active| active.set(control));
}

/// True when `control`'s rule is deleted on this thread.
#[cfg(serialize_controls)]
#[inline]
pub(crate) fn deleted(control: Control) -> bool {
    ACTIVE.with(|active| active.get() == Some(control))
}

/// True when `control`'s rule is deleted on this thread: never, outside a controls build.
#[cfg(not(serialize_controls))]
#[inline(always)]
pub(crate) const fn deleted(_control: Control) -> bool {
    false
}
