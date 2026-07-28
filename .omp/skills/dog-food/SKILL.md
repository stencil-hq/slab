---
name: dog-food
description: "Run an exhaustive parallel dogfooding sweep for a library, language, SDK, or runtime: send three independent agents to build the same ambitious application on different supported platforms, drive each app through its real automation surface, capture friction reports and minimal repros, consolidate every finding into a coverage matrix, then dispatch 4–8 subsystem agents to fix all concerns and validate end to end. Use when asked to dogfood, stress-test developer experience, identify library/runtime friction, build the same app across web/TUI/native, or turn a multi-platform UX audit into a complete fix sweep."
---

# Dog-food

Dogfood the product twice: first as an external user discovering friction, then as its owner removing every discovered concern.

## Workflow

1. Define one ambitious app brief and three supported platforms.
2. Launch three independent builder agents in one batch.
3. Make each builder run and drive its app through the real platform interface.
4. Collect structured reports, repros, recordings, and screenshots.
5. Concatenate every finding into an exhaustive coverage matrix.
6. Launch 4–8 subsystem fix agents in one batch; assign every matrix row.
7. Integrate once, run the complete validation matrix, and fix all failures.
8. Re-run the repros and core app flows against the fixed product.
9. Deliver the coverage matrix with evidence for every resolution.

## Non-negotiable rules

- Keep the three builder agents independent until all reports are complete. Independent corroboration is evidence; cross-pollination destroys it.
- Give every builder the same product brief, ambition level, documentation entry point, reporting schema, and feature menu.
- Builders act as external consumers. They MUST work outside the product repository, normally under `/tmp`, and MUST NOT fix the library while testing it.
- Make builders use the distribution path a real user would use: registry package, released binary, or documented Git dependency. Record exact versions/revisions. If testing local HEAD instead, state that deviation explicitly.
- Drive the running artifact, not merely a test file. Use the platform's actual automation/debug surface.
- Preserve unsuccessful attempts. An impossible feature, dead end, misleading diagnostic, or undocumented workaround is a finding.
- Assign every report row in the fix wave. “External,” “intentional,” “minor,” or “out of scope” does not permit omission.
- Treat intentional design limitations as product decisions to improve, not mislabeled bugs. Change the spec when the friendliest coherent design requires it.
- Fix agents MUST skip formatters, builds, linters, tests, generation, and project-wide validation while editing concurrently. The parent runs these once after the wave settles.
- Do not declare completion until original repros and platform flows pass against the fixed product.

## Phase 1: Scope the experiment

### Choose one shared app

Pick a familiar app whose behavior stresses most of the product surface. A TODO app is effective because it naturally exercises:

- CRUD and in-place editing
- Lists, stable identity, filtering, search, and history
- Keyboard and pointer input
- Focus, scrolling, virtualization, overlays, and responsive layout
- Themes and interaction states
- Timers, reminders, recurring state, and live updates
- Accessibility and automation semantics
- Packaging, code generation, and host APIs

Give all builders the same required core, then tell them to go beyond it: reminders, flags, priorities, timers, recurring tasks, historical views, themes, shortcuts, bulk operations, long/unicode content, large lists, rapid input, and anything else that tests a boundary.

### Choose three representative platforms

Prefer materially different clients, for example:

1. Browser/web component or framework integration
2. Terminal/TUI integration
3. Native GPU/windowed integration

For each platform, identify before spawning:

- Primary docs and normative specs
- Package/repository dependency path
- Canonical examples
- Required driver: browser automation, TUI recording/debug skill, native drive/debug protocol, etc.
- Expected build/run command

### Allocate isolated paths

Generate one epoch and use stable, obvious directories:

```text
/tmp/dog-food-<epoch>-web/
/tmp/dog-food-<epoch>-tui/
/tmp/dog-food-<epoch>-native/
```

Use the app name instead of `dog-food` when useful. Never let builders share application files.

## Phase 2: Launch the builder wave

Launch all three agents in one `task` batch. Do not spawn one and wait. Give shared context once, then a platform-specific target/change/acceptance block to each agent.

### Shared builder contract

Require every builder to:

1. Read the supplied docs before source archaeology.
2. Scaffold a real external project in its assigned `/tmp` directory.
3. Keep application state/policy in the documented host layer and product-owned behavior in the library/runtime layer.
4. Build the core app, then deliberately attempt ambitious and edge-case features.
5. Run the documented checker/compiler after source edits.
6. Keep a live `NOTES.md` friction journal while working.
7. Launch the app through a managed process when it is long-running.
8. Drive every major flow through the required platform tool.
9. Save screenshots, recordings, transcripts, scene dumps, and minimal repros beside the app.
10. Write `REPORT.md` using the schema below.

Do not instruct builders to read other builders' notes or reports.

### Platform driving requirements

- **Web:** open the live page with the browser tool; inspect the accessibility/semantic tree; click, type, tab, scroll, exercise timers, and capture screenshots. Test empty, long-text, unicode, many-item, and rapid-input states. For persistent screenshot artifacts, use raw Puppeteer `page.screenshot({ path })` and verify that the requested path exists; the `tab.screenshot({ path })` helper may redirect to a temporary path.
- **TUI:** read and follow the `tui-debug` skill. Use its runner/workflow rather than invoking the underlying recorder ad hoc. Record scripted CRUD, navigation, editing, scrolling, and timer flows; retain checkpoint images and terminal output.
- **Native:** mount the product's drive/debug protocol on the live instance when available. Drive scene queries, input, text, clock, theme, scrolling, and rendering through that protocol. Capture protocol transcripts and authoritative offscreen renders in addition to OS screenshots.

If a platform lacks a usable driver, that is a major finding. Build the smallest truthful workaround and document the missing product surface.

## Builder report schema

Require these sections in every `REPORT.md`:

1. **What I built** — features, architecture, host/runtime responsibility split, artifact paths.
2. **Session timeline** — major steps, dead ends, and time sinks.
3. **Friction log** — one numbered entry per concern:
   - what I tried
   - what happened
   - what I expected
   - severity: blocker / major / minor / paper-cut
   - layer: language / compiler / runtime / client API / docs / packaging / external tooling
   - workaround, if any
   - concrete suggested fix
4. **What worked well** — behavior and APIs worth preserving.
5. **Bugs and minimal repros** — exact files and commands.
6. **Top recommendations** — ranked by new-user impact.

A report is incomplete if it only summarizes the successful result.

## Phase 3: Build the exhaustive coverage matrix

After all builders finish:

1. Verify that each app, report, and claimed artifact exists.
2. Read all reports in parallel.
3. Concatenate every numbered row; do not summarize rows away.
4. Deduplicate only by linking duplicate evidence to one canonical concern. Preserve every source row in the matrix.
5. Classify each concern accurately:
   - confirmed implementation bug
   - intentional design limitation requiring a product/spec decision
   - documentation or example defect
   - released-artifact/source version skew
   - packaging/tooling defect
   - external harness or third-party issue
6. Record the owning subsystem and required observable resolution.
7. Preserve independent corroboration, repro paths, and positive feedback.

Use a matrix with at least:

```text
source row | concern | evidence/repro | classification | owner | required resolution | status
```

For an external issue, require all three of:

1. Minimal deterministic repro
2. Upstream-ready issue or patch
3. Local documentation or workflow mitigation

Never label an external concern “excluded.”

## Phase 4: Launch the fix wave

Scope the repository and split by real subsystem boundaries. Launch 4–8 agents in one batch. Typical slices:

- Language, compiler, and normative spec
- Runtime state, focus, input, and layout
- Text/font/rendering fidelity
- Web/WASM client API
- TUI host API
- Native host/renderer shell
- Drive/debug protocol
- CLI, code generation, packaging, and documentation

### Fix-wave shared contract

Provide every fixer:

- The complete coverage matrix URI
- Its exact row IDs
- Referenced repro/report paths
- Files/symbols it owns
- Explicit non-goals
- Cross-task API contracts
- The instruction to enumerate every assigned row in its completion report

Define cross-subsystem contracts before dispatch. Name one source-of-truth owner for shared representations such as frame diagnostics, token values, scene-key grammar, reload invalidation, or generated bindings. Tell consumers to coordinate through `hub` and never invent competing APIs.

Fixers must implement, document, and add focused regression coverage in one pass, but MUST NOT run project-wide validation during the concurrent wave. They should report generated artifacts the parent must refresh.

## Phase 5: Integrate and validate

The parent owns integration after every fixer has stopped editing.

Run in dependency order:

1. Regenerate protobufs, grammars, capability tables, bindings, and checked-in artifacts.
2. Run format and static/lint checks.
3. Run focused failing repros as failures appear.
4. Run the complete workspace/unit/integration test suite.
5. Run native and cross-host conformance; review semantic golden changes before updating them.
6. Run freshness/generated-artifact checks.
7. Build/package released artifacts.
8. Run browser, TUI, and native end-to-end smoke paths.
9. Assert that every cited screenshot, recording, transcript, and render path exists; a tool success message alone is not artifact evidence.
10. Re-run every original minimal repro.
11. Rebuild or relink the three dogfood apps against the fixed source/packages and drive their core flows again.

Fix failures at the source. Do not refresh goldens merely to hide semantic drift. Repeat the failing focused check, then the full gate it belongs to.

Also audit the touched public surface:

- Every public symbol documented
- No stale aliases, shims, or duplicate conventions
- Generated APIs match runtime APIs
- Docs, examples, and released package behavior agree
- Diagnostics are actionable and observable on every client
- No platform driver independently implements kernel-owned behavior

## Phase 6: Deliver

Return an evidence-first summary containing:

- Paths to all three apps and reports
- The exhaustive coverage matrix
- Resolution of every row: code fix, spec redesign, docs fix, packaging fix, or upstream issue plus mitigation
- Important API/spec decisions
- Generated artifacts updated
- Exact validation and dogfood commands that passed
- Positive behaviors deliberately preserved
- Any genuinely unreachable external prerequisite and everything attempted

Do not report a plausible subset as complete. The coverage matrix is the completion definition.

## Useful orchestration pattern

```text
Parent scopes brief + platforms + contracts
  ├─ Builder Web    ┐
  ├─ Builder TUI    ├─ independent reports + evidence
  └─ Builder Native ┘
Parent creates exhaustive matrix
  ├─ Fix compiler/spec
  ├─ Fix runtime/input
  ├─ Fix rendering
  ├─ Fix web
  ├─ Fix TUI/native
  └─ Fix drive/CLI/docs
Parent regenerates → validates → re-dogfoods → delivers matrix
```

The parent interprets user intent, owns decomposition, and resolves integration. Subagents execute independent slices; they do not replace top-level product judgment.
