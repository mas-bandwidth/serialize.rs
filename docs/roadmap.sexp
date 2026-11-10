; The serialize.rs roadmap: the data ROADMAP.md is generated from.
; Same shape as nova-sprint's docs/roadmap.sexp (internal/roadmapdoc's package comment):
; data, never evaluated. Edit this file, then regenerate ROADMAP.md with nova-sprint's
; generator, from a nova-sprint checkout:
;   go build -o /tmp/roadmap ./tools/roadmap && (cd <this repo> && /tmp/roadmap --file docs/roadmap.sexp --out ROADMAP.md)
; Never edit ROADMAP.md by hand. Created 2026-10-14 when this repository's open GitHub issues and
; unfinished pull requests were moved into it.
(roadmap "v1"
 :title "serialize.rs roadmap"
 :text "Planned work for serialize.rs. The items below were open GitHub issues and pull requests, moved here on 2026-10-14 when roadmaps became the standard for future work (2026-10-14). Each item names its issue or pull request; the branches of moved pull requests are kept to resume from. Landed work is in git."

 :groups
 (
  (group "github-issues" :title "Moved from GitHub issues on 2026-10-14"
   :text "Open GitHub issues of mas-bandwidth/serialize.rs, recorded here on 2026-10-14 when roadmaps became the standard for future work (2026-10-14). Each item names its issue; the issue is closed with a pointer here.")
 )

 :items
 (
  (item "issue-78" :group "github-issues" :area "serialize.rs"
   :title "Keep ordinary change CI under two minutes; schedule extended fuzz runs"
   :text "The image-only README logo change in PR77 ran the entire fuzz smoke job (60s per target), exceeding the per-change two-minute ceiling while no Rust source changed. Extended fuzzing now runs on a nightly schedule in `.github/workflows/nightly-fuzz.yml`; ordinary changes still run the two 60s smoke targets in `.github/workflows/ci.yml`, so the remaining work is to make that gate paths-aware or otherwise keep ordinary change CI under two minutes."
   :date "2026-10-14"
   :origin "mas-bandwidth/serialize.rs issue #78, opened 2026-09-13 by gafferongames; moved here 2026-10-14"
   :kept (:github-issue "78" :labels ""))
 ))
