# ADR-0003: Let a program measure and block, and a skill judge and propose

- **Status:** accepted
- **Date:** 2026-10-24
- **Type (ISO/IEC/IEEE 14764):** preventive
- **Decided by:** Giovanni (owner) / proposed by Claude Code

## Context

The quality gate is strong, but it holds only because Claude remembers to run it. Mature teams make the machine
refuse the merge. The existing checks are also scattered (separate fitness and metrics scripts, the gate script) and the
architecture check only searched the Gradle file, not the source.

## Decision

Two pieces that talk only through one file, `report.json`:

- `pipeline-check` (Rust, `tools/pipeline-check/`) measures and blocks. It is deterministic, runs in CI without
  Claude, and exits non-zero when a check is `falha`. Limits live in `pipeline-check.toml`, decided by Giovanni.
- The skill `revisao-manutencao` reads `report.json`, never re-measures, and proposes `debt` issues, priorities and a
  Preventive decision-log row. It proposes; Giovanni decides.

The program replaces ad-hoc fitness and metrics scripts. Blocking a merge also needs branch
protection that requires the CI job; that setting is changed only by Giovanni.

## Alternatives considered

- **Extend the Python scripts** — the language was Giovanni's call (work.ask-language): Rust.
- **Let the skill measure** — judgement and measurement mix, and nothing blocks when Claude forgets.
- **Python/Shell checks only in the gate** — no single report, no warning band.

## Update 2026-10-24

The program is not copied into projects: it lives in `yujideluca/pipeline-check` and a project pins a tag in `pipeline-check.version`
(ADR-0003 of that repository gives the reason and the rule: a tool used by more than one project lives once, consumed by version).

## Consequences

CI builds a Rust program on each run (cached). Limits change only in a commit with a decision-log row. REQ PC-1..PC-11
are checked by the tests of the crate; the traceability gate scans them.
