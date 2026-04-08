# Changelog

## 0.1.0 (2026-04-08)

Initial release.

- Core types: DAG, agents, Git events, config, feedback
- Agent lifecycle: Nix environments, git worktrees, harness with heartbeat
- Supervisor: health monitor, intervention playbook, escalation gateway, watchdog
- DAG engine: loader, validator, state machine, dependency resolution
- Git monitor: event handler dispatch, canonical completion chain
- Orchestrator: main loop wiring all components
- PO Agent: repo analysis, DAG generation, issue creation
- Retro engine: KPIs, improvements, learnings store
- Noether integration: CLI client for stage search/compose/run
- Agent generator: 4-axis composition (personality x capabilities x model x framework)
- Dashboard: per-project status, cross-project view
- MkDocs documentation (13 pages)
- 192 tests
