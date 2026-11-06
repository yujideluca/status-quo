# Maintenance and evolution

The method: maintenance types of **ISO/IEC/IEEE 14764** (corrective, adaptive, perfective, preventive), **Lehman's
laws** (software in use must keep changing, and its complexity grows unless work is spent to reduce it),
**evolutionary architecture** (architecture rules as automatic checks, "fitness functions") and the **DORA**
delivery metrics (how often and how fast changes reach users, how often they need repair).

## 1. Technical debt

**Debt** = anything that works today but makes the next change slower or riskier: a missing test, a known
limitation, a file doing too many jobs, a manual step that should be a script.

- Each item is a GitHub Issue labelled `debt` with one priority: `P1` (risk of data loss or wrong results),
  `P2` (slows every change or misleads the user), `P3` (nice to have). New features use the label `idea`.
- **Fixed share:** about one work session in five (≈ 20 %) goes to debt or prevention, highest priority first.
- A limitation found while working becomes an issue in the same session.

## 2. Monthly architecture review (all projects)

One session a month; it is an open item in workflow-memory, so every chat shows it until done. For each active
project in `~/dev`:

0. Run the skill `revisao-manutencao`: it runs `tools/pipeline-check.sh`, reads `build/pipeline-check/report.json`, proposes `debt` issues and the Preventive row, and never measures by hand (ADR-0003). Steps 1-3 below are what the program already measured; the rest is judgement.
1. The gate passes locally and the latest CI runs on GitHub are green (`gate`, `ci-verde` in the report).
2. Metrics (the `dora` block of the report): deployment frequency, lead time, rework rate (share of
   Corrective changes), preventive share (target ≈ 20 %).
3. Fitness functions (`pipeline-check.toml`, warning from 80 %, failure at 100 %): what is in `aviso`? Act before it crosses.
4. `DESIGN.md` diagrams and the ADRs still match the code; every structural change since the last review has an ADR.
5. Adaptive maintenance: toolchain, SDK and dependency updates due? Upgrade on a branch, run the gate.
6. Debt list: still true, right priority; a `P1` older than a month is scheduled now.
7. Across projects: the same problem solved twice in different ways? Move the better solution into this template.
8. Memory: are workflow-memory entries still true (`mem search <project>`)? Outdated → `mem feedback REF stale`.

Record the outcome as a decision-log row of Type *Preventive* in each project reviewed, then close the open item
with `mem use REF --note "..."` (it stays open as the monthly reminder).

## 3. The machine that blocks (pipeline-check)

`pipeline-check` (Rust, ADR-0003; its own repository `yujideluca/pipeline-check`, pinned in `pipeline-check.version`) is the one program that measures: it runs the gate once, checks the
architecture rule from the source, evaluates the fitness functions, computes the DORA figures and, on the laptop,
reads CI and branch protection through `gh`. Limits live in `pipeline-check.toml` and change only on Giovanni's
decision. CI runs it instead of the bare gate, so a `falha` fails the job. Repository protection must require every CI job that checks a requirement before promotion to `main`: `gate` and `windows` (SQ-36; a job that runs but is not required can stay red unnoticed, as `windows` did from b701cda to 2026-11-06); the local pre-push check (`tools/hooks/pre-push`) supplements that protection. Bootstrap the repository with an exact-commit green CI run, then enforce the required check before subsequent promotion. If the account cannot protect a private repository, resolve the repository visibility/plan decision without bypassing protection. The first review of a project also creates its bug-finding test suite
(skill `gerar-teste-automatizado`, record `docs/TESTES-AUTOMATIZADOS.md`); later reviews update it.

## 7. Desktop end-to-end test (tray)

`tools/ui_e2e.sh` runs `crates/desktop/tests/desktop_e2e.rs` against the production build on the real desktop. It pauses the live tray, starts a private copy (own state directory and collector), drives it with a virtual mouse made through `/dev/uinput`, and checks what the tray saves: start, idle CPU and memory, Edit tray and Cancel, move, corner and edge resize, Done and docking, token controls, Add widget, Customize and Cancel, and a restart. It writes screenshots and `report.json` to `build/e2e/` and restarts the live tray afterwards. It skips with a stated reason when there is no Wayland session, `/dev/uinput` is not writable or `spectacle` is missing.

- **The screen must be free:** the virtual mouse clicks on whatever window is in front (a run once hit a maximized browser). The runner refuses unless `STATUS_QUO_E2E_SCREEN_IS_FREE=1` is set; close or minimize windows over the right side of the screen and leave the mouse alone for about three minutes first.
- **When:** before closing any major tray update (layout, editing, host surface, widget rendering, configuration shape) and before a release. `STATUS_QUO_E2E=1 STATUS_QUO_E2E_SCREEN_IS_FREE=1 tools/quality_gate.sh` runs it as gate step 6.
- **Keeping it current:** the expected positions and numbers live in `crates/desktop/tests/e2e/manual-baseline.json`, recorded from the manual test of 2026-11-02. When a major update changes the tray, repeat the manual pass, update the baseline and extend the test in the same change; a feature without an end-to-end check is not finished. Findings that are known but unfixed are listed under `open_findings` in the baseline and copied into every report.
- **Limits:** one screen size (1920x1080) and the first monitor; button positions are recorded values, not found on screen; the idle check lasts 20 seconds, not the 5 minutes of SQ-23.
