; The serialize.rs roadmap: the data ROADMAP.md is generated from.
; Same shape as nova-sprint's docs/roadmap.sexp (internal/roadmapdoc's package comment):
; data, never evaluated. Edit this file, then regenerate ROADMAP.md with nova-sprint's
; generator, from a nova-sprint checkout:
;   go build -o /tmp/roadmap ./tools/roadmap && (cd <this repo> && /tmp/roadmap --file docs/roadmap.sexp --out ROADMAP.md)
; Never edit ROADMAP.md by hand. Created 2026-10-14 when this repository's open GitHub issues and
; unfinished pull requests were moved into it.
(roadmap "v1"
 :title "serialize.rs roadmap"
 :text "Planned work for serialize.rs. The items below were open GitHub issues and pull requests, moved here on 2026-10-14 when roadmaps became the standard for future work (Glenn, 2026-10-14). Each item names its issue or pull request; the branches of moved pull requests are kept to resume from. Landed work is in git."

 :groups
 (
  (group "github-issues" :title "Moved from GitHub issues on 2026-10-14"
   :text "Open GitHub issues of mas-bandwidth/serialize.rs, recorded here on 2026-10-14 when roadmaps became the standard for future work (Glenn, 2026-10-14). Each item names its issue; the issue is closed with a pointer here.")
  (group "uniform-rules" :title "Uniform rules left after the 2.1.1 rulings"
   :text "What the 2.1.1 rulings (IMPLEMENTING.md U6, U9, U15, serialize v2.1.1) ask of this port that a patch release cannot give: each needs a ruling or a major version. Landed in 4.0.1: the measure value-width trap (U6), the U9 trap documented and named, write misuse never latching, the U15 copy_string vectors.")
 )

 :items
 (
  (item "issue-78" :group "github-issues" :area "serialize.rs"
   :title "Keep ordinary change CI under two minutes; schedule extended fuzz runs"
   :text "The image-only README logo change in PR77 ran the entire fuzz smoke job (60s per target), exceeding the per-change two-minute ceiling while no Rust source changed. Keep useful source-change gates, but move the multi-target extended fuzz work to scheduled/explicitly invoked CI and use a small bounded smoke or paths-aware gate for ordinary changes."
   :date "2026-10-14"
   :origin "mas-bandwidth/serialize.rs issue #78, opened 2026-09-13 by gafferongames; moved here 2026-10-14"
   :kept (:github-issue "78" :labels ""))
  (item "u9-write-fail-false" :group "uniform-rules" :area "serialize.rs"
   :title "U9: fail, reject and a failing nested object on a write or measure have no false to return"
   :text "U9 (2.1.1) has fail on a write or measure return false, a nested object whose serialize returns false make the object operation return false, and reject return false in an unchecked build. Since 2.0 (the #52 ruling) this port's WriteStream and MeasureStream error type is Infallible, so their result type has no false: each returns Ok(()), records nothing and never panics. A serialize function that returns through fail returns there, but its caller's ? continues, where a C++ caller would see false and stop. Giving the write path a false means a fallible write error type, which reverses #52 and breaks every implementor's write signature: a major version, and Glenn's call. Done when: a ruling recorded with Glenn's words: either U9 accepts an uninhabited write error type as this port's false (IMPLEMENTING.md or a decision file says so), or a 5.0.0 gives WriteStream and MeasureStream a fallible error type and tests/uniform.rs proves fail, reject (release) and a failing nested object return Err on both streams, recording nothing."
   :date "2026-10-27"
   :origin "IMPLEMENTING.md U9 (serialize v2.1.1); left at the 4.0.1 rulings landing")
  (item "u15-exports" :group "uniform-rules" :area "serialize.rs"
   :title "U15: hide or rule on the exports beyond the reference"
   :text "The 2.1.1 exports ruling allows the reference operations, the three streams, the refusal list and its name function, copy_string, the U15 extras, the reference's public helpers and documented language necessities; anything else is hidden where the language can hide it. Beyond that list this port exports Error::from_name (the inverse of the name function), Stream::refuse_if_failed (the zero-bit failure gate a custom stream implements) and Stream::refuse (the latching form of fail). FixedPointStorage is documented as a language necessity, and the controls (Control, set_control) exist only under --cfg serialize_controls, doc(hidden). Hiding a public item is a breaking change, so it waits on a major version or a ruling. Done when: each named export is either allowed by a recorded ruling (and documented as such where it is defined) or hidden in a major release, with cargo-semver-checks seeing the break as intended."
   :date "2026-10-27"
   :origin "IMPLEMENTING.md U15 exports ruling (serialize v2.1.1); left at the 4.0.1 rulings landing")
 ))
