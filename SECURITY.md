# Security Policy

serialize.rs is a bit-packing serialization library. It reads untrusted data — a decoder is
handed bytes that arrived from somewhere else, and a malformed or hostile stream must be
rejected rather than trusted, panicked on, or read past.

## Reporting a vulnerability

**Please do not report security issues in public GitHub issues or pull requests.**

Report privately through either channel:

- **GitHub private vulnerability reporting** (preferred): on this repository, go to the
  **Security** tab → **Report a vulnerability**. This opens a private advisory visible only
  to the maintainers.
- **Email**: glenn@mas-bandwidth.com.

Please include enough detail to reproduce: the affected version or commit, a description of
the flaw, and — where possible — a proof-of-concept input or a small patch.

We will acknowledge your report, keep you updated on our assessment, and coordinate
disclosure timing with you. We prefer coordinated disclosure and will credit reporters who
wish to be named.

## Scope

In scope — bugs in this crate, above all on the READ path: a stream that causes a read past
the end of the buffer, a panic, an unbounded allocation, or a value accepted outside the
range the schema declared.

The crate forbids unsafe code crate-wide — `unsafe_code = "forbid"` under `[lints.rust]` in
`Cargo.toml` — so classic memory corruption is off the table by
construction; the interesting class is a hostile stream that is *accepted* when it should be
rejected, or that panics and takes the process down.

Note the deliberate asymmetry, which is the same as the C library's: on WRITE the caller is
responsible for passing sane values, and debug assertions are a development aid rather than
a runtime guard. On READ the library checks, because that is where untrusted data arrives.

The wire format is specified in `STANDARD.md`. **A flaw in the *specification* — as opposed
to this implementation of it — is in scope and is more valuable to us**, because it affects
every implementation of serialize rather than one. Report those the same way.

## No known vulnerabilities

serialize has no published security advisories. The AEAD nonce-reuse issue that affected the
netcode family in July 2026 does not apply here: this library does not handle keys or
encryption.

## Supported versions

Security fixes land on the latest release. We do not backport to older release lines.
