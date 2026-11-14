# rsStudio development plan

rsStudio is the working name for an independently developed, Linux-first R IDE. Rust and egui/eframe provide the desktop application; an existing R installation executes R code. The first milestone is a responsive GUI connected to one persistent R session. A phase 1 prototype is in progress; its GUI and real-R integration still require end-to-end validation. Later-phase features remain proposed.

## Reading order

1. [Roadmap and delivery gates](roadmap.md) — scope, dependencies, and release definition.
2. [Architecture and R runtime](architecture.md) — process boundaries, execution protocol, and lifecycle.
3. [UX and themes](ux-and-themes.md) — workspace, visual direction, and Omarchy integration.
4. [Quality, security, and delivery](quality-and-delivery.md) — verification, packaging, and licensing.
5. Phase briefs: [01 Foundation](phases/01-foundation.md), [02 Authoring](phases/02-authoring.md), [03 Inspect and plot](phases/03-inspect-and-plot.md), [04 MVP release](phases/04-mvp-release.md), and [05 Later platforms](phases/05-later-platforms.md).

## Planning conventions

- Each phase has a demo, deliverables, and verifiable exit criteria. Tasks are proposals until implemented; no dates or estimates are assumed.
- Deliver a narrow end-to-end slice before expanding features. A phase is complete only when its exit criteria and applicable checks pass.
- Keep code and design original. Refer to public documentation for user workflows, not implementation or branding. Clear the working name before public release; original project files use the MIT License.
- Owners should record decisions and implementation changes in the relevant document as the team learns from prototypes.
