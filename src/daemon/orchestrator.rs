use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::Mutex;

use caloron_types::config::CaloronConfig;

use crate::agent::spawner::AgentSpawner;
use crate::dag::engine::DagEngine;
use crate::daemon::socket::DaemonSocket;
use crate::daemon::state::DaemonState;
use crate::git::GitHubClient;
use crate::git::monitor::{EventHandler, OrchestratorAction};
use crate::supervisor::health_monitor::{HealthMonitor, HealthMonitorConfig};
use crate::supervisor::interventions::{InterventionDecider, InterventionTracker, InterventionAction};

/// The main orchestration loop that ties all components together.
pub struct Orchestrator {
    config: CaloronConfig,
    state: DaemonState,
    dag: Arc<Mutex<DagEngine>>,
    github: GitHubClient,
    spawner: Arc<Mutex<AgentSpawner>>,
    health_monitor: HealthMonitor,
    intervention_tracker: InterventionTracker,
}

impl Orchestrator {
    /// Create a new orchestrator from configuration and a loaded DAG.
    pub fn new(
        config: CaloronConfig,
        dag: DagEngine,
        github: GitHubClient,
        repo_root: PathBuf,
        socket_path: PathBuf,
    ) -> Self {
        let state = DaemonState::new(config.clone());

        let spawner = AgentSpawner::new(config.clone(), repo_root, socket_path);

        let health_config = HealthMonitorConfig {
            check_interval: Duration::from_secs(config.github.polling_interval_seconds as u64),
            max_review_cycles: config.supervisor.max_review_cycles,
            ..Default::default()
        };

        Self {
            config,
            state,
            dag: Arc::new(Mutex::new(dag)),
            github,
            spawner: Arc::new(Mutex::new(spawner)),
            health_monitor: HealthMonitor::new(health_config),
            intervention_tracker: InterventionTracker::new(),
        }
    }

    /// Run the main orchestration loop.
    pub async fn run(&mut self) -> Result<()> {
        let sprint_id = {
            let dag = self.dag.lock().await;
            dag.sprint_id().to_string()
        };
        tracing::info!(sprint_id, "Starting orchestration loop");

        // Ensure labels exist
        if let Err(e) = self.github.ensure_labels().await {
            tracing::warn!(error = %e, "Could not ensure labels — continuing anyway");
        }

        // Spawn agents for initially ready tasks
        self.spawn_ready_tasks().await?;

        // Start the daemon socket for harness communication
        let socket_path = PathBuf::from(format!("/run/caloron/{sprint_id}.sock"));
        let socket = DaemonSocket::new(socket_path, self.state.clone());
        let socket_handle = tokio::spawn(async move {
            if let Err(e) = socket.listen().await {
                tracing::error!(error = %e, "Socket server error");
            }
        });

        let poll_interval = Duration::from_secs(
            self.config.github.polling_interval_seconds as u64,
        );

        // Main loop: poll events, handle them, check health
        loop {
            // Poll for new Git events (coalesced — all events per cycle, per Addendum H4)
            match self.github.poll_events().await {
                Ok(events) => {
                    for event in events {
                        if let Err(e) = self.handle_event(&event).await {
                            tracing::error!(error = %e, ?event, "Failed to handle event");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to poll events");
                }
            }

            // Run health checks
            self.run_health_checks().await;

            // Check if sprint is complete
            {
                let dag = self.dag.lock().await;
                if dag.is_sprint_complete() {
                    tracing::info!(sprint_id, "Sprint complete!");
                    break;
                }
            }

            tokio::time::sleep(poll_interval).await;
        }

        socket_handle.abort();
        Ok(())
    }

    /// Handle a single Git event.
    async fn handle_event(&mut self, event: &caloron_types::git::GitEvent) -> Result<()> {
        let action = {
            let mut dag = self.dag.lock().await;
            EventHandler::handle(event, &mut dag)?
        };

        self.execute_action(action).await
    }

    /// Execute an orchestrator action. Flattens Multiple into a sequential list.
    async fn execute_action(&mut self, action: OrchestratorAction) -> Result<()> {
        // Flatten Multiple into a sequential queue to avoid async recursion
        let actions = match action {
            OrchestratorAction::Multiple(inner) => inner,
            single => vec![single],
        };
        for action in actions {
            self.execute_single_action(action).await?;
        }
        Ok(())
    }

    async fn execute_single_action(&mut self, action: OrchestratorAction) -> Result<()> {
        match action {
            OrchestratorAction::None => {}

            OrchestratorAction::SpawnAgent {
                task_id,
                agent_def_id,
                issue_number,
            } => {
                tracing::info!(task_id, agent_def_id, issue_number, "Spawning agent for task");

                // Add assignment label and comment
                let _ = self.github.add_label(issue_number, "caloron:assigned").await;
                let comment = format!("@caloron-agent-{agent_def_id} has been assigned this task.");
                let _ = self.github.create_comment(issue_number, &comment).await;

                // TODO: Load agent definition, spawn via spawner
                // This requires loading the YAML from caloron-meta and calling spawner.spawn()
                // For now, log the intent
                tracing::info!(agent_def_id, task_id, "Agent spawn requested");
            }

            OrchestratorAction::AssignReviewer {
                pr_number,
                reviewer_agent_id,
            } => {
                tracing::info!(pr_number, reviewer_agent_id, "Assigning reviewer");
                let _ = self.github.add_label(pr_number, "caloron:review-pending").await;
            }

            OrchestratorAction::MergePr { pr_number } => {
                tracing::info!(pr_number, "Auto-merging PR");
                if let Err(e) = self.github.merge_pr(pr_number).await {
                    tracing::error!(pr_number, error = %e, "Failed to merge PR");
                }
            }

            OrchestratorAction::NotifyAgent {
                agent_id,
                issue_number,
                message,
            } => {
                tracing::info!(agent_id, issue_number, "Notifying agent");
                // Reset stall timer
                self.state.update_agent(&agent_id, |health| {
                    health.record_git_event();
                }).await;
            }

            OrchestratorAction::StoreFeedback { task_id, feedback } => {
                tracing::info!(task_id, "Storing feedback for retro");
                // TODO: Store in retro buffer (Phase 6)
            }

            OrchestratorAction::TaskCompleted {
                task_id,
                issue_number,
                pr_number,
                unblocked,
            } => {
                tracing::info!(task_id, pr_number, "Task completed");

                // Completion chain (Addendum E2):
                let _ = self.github.add_label(issue_number, "caloron:done").await;
                let comment = format!("Completed via PR #{pr_number}");
                let _ = self.github.close_issue(issue_number, &comment).await;

                // Spawn agents for newly unblocked tasks
                for unblocked_id in unblocked {
                    tracing::info!(unblocked_id, "Task unblocked");
                    // These will be picked up on the next poll when issues are created
                }
            }

            OrchestratorAction::TaskRework {
                task_id,
                issue_number,
                author_agent_id,
            } => {
                tracing::info!(task_id, author_agent_id, "Task needs rework");
                let _ = self.github.remove_label(issue_number, "caloron:review-pending").await;
                let _ = self.github.remove_label(issue_number, "caloron:changes-requested").await;
                let comment = format!(
                    "PR was closed without merge. @caloron-agent-{author_agent_id}: please review the closure reason and rework."
                );
                let _ = self.github.create_comment(issue_number, &comment).await;
            }

            OrchestratorAction::NotifySupervisor { reason } => {
                tracing::warn!(reason, "Supervisor notification");
            }

            OrchestratorAction::Multiple(_) => {
                unreachable!("Multiple actions are flattened in execute_action")
            }
        }

        Ok(())
    }

    /// Spawn agents for all currently ready tasks.
    async fn spawn_ready_tasks(&mut self) -> Result<()> {
        let ready: Vec<String> = {
            let dag = self.dag.lock().await;
            dag.get_ready_tasks()
                .iter()
                .map(|ts| ts.task.id.clone())
                .collect()
        };

        for task_id in ready {
            tracing::info!(task_id, "Task is ready — awaiting issue creation to spawn agent");
        }

        Ok(())
    }

    /// Run health checks and execute interventions.
    async fn run_health_checks(&mut self) {
        let agents = self.state.all_agent_health().await;
        let results = self.health_monitor.check_all(&agents);

        for result in results {
            let task_id = agents
                .get(&result.agent_id)
                .and_then(|h| h.current_task_id.clone())
                .unwrap_or_default();

            let action = InterventionDecider::decide(
                &result,
                &task_id,
                &self.intervention_tracker,
            );

            tracing::warn!(
                agent_id = result.agent_id,
                verdict = ?result.verdict,
                intervention = ?action,
                "Health check failed"
            );

            // Record the intervention
            self.intervention_tracker.record(
                &task_id,
                &result.agent_id,
                action.clone(),
            );

            // Execute the intervention
            match action {
                InterventionAction::Probe => {
                    if let Some(health) = agents.get(&result.agent_id) {
                        if let Some(issue) = health.current_task_id.as_ref().and_then(|_| {
                            // TODO: get issue number from DAG
                            None::<u64>
                        }) {
                            let minutes = (chrono::Utc::now() - health.last_git_event)
                                .num_minutes() as u64;
                            let comment = crate::supervisor::interventions::probe_comment(
                                &result.agent_id,
                                minutes,
                            );
                            let _ = self.github.create_comment(issue, &comment).await;
                        }
                    }
                }
                InterventionAction::Restart => {
                    tracing::info!(
                        agent_id = result.agent_id,
                        "Restarting agent"
                    );
                    // TODO: Call spawner.restart() with agent definition and credentials
                }
                InterventionAction::EscalateCredentials { tool } => {
                    let escalation = crate::supervisor::escalation::EscalationIssue::CredentialsFailure {
                        agent_role: result.agent_id.clone(),
                        tool,
                        task_issue_number: 0, // TODO: look up from DAG
                        error_count: agents.get(&result.agent_id).map(|h| h.consecutive_errors).unwrap_or(0),
                    };
                    let _ = crate::supervisor::escalation::EscalationGateway::escalate(
                        &self.github,
                        &escalation,
                    ).await;
                }
                InterventionAction::EscalateCapability => {
                    tracing::error!(
                        agent_id = result.agent_id,
                        "Task beyond agent capability — escalating"
                    );
                }
                InterventionAction::EscalateReviewLoop { pr_id } => {
                    tracing::warn!(pr_id, "Review loop — escalating");
                }
                InterventionAction::Reassign { new_agent_id } => {
                    tracing::info!(
                        from = result.agent_id,
                        to = new_agent_id,
                        "Reassigning task"
                    );
                }
                InterventionAction::Block { reason } => {
                    tracing::warn!(
                        agent_id = result.agent_id,
                        reason,
                        "Blocking task"
                    );
                    if !task_id.is_empty() {
                        let mut dag = self.dag.lock().await;
                        let _ = dag.task_blocked(&task_id, &reason);
                    }
                }
            }
        }
    }
}

/// Entry point: load config and DAG, create orchestrator, and run.
pub async fn start_daemon(config_path: &Path, dag_path: &Path) -> Result<()> {
    let config = crate::config::load_config(config_path)?;

    let dag = DagEngine::load_from_file(dag_path)
        .context("Failed to load DAG")?;

    let sprint_id = dag.sprint_id().to_string();

    // Set up state persistence
    let state_file = format!("state/sprint-{sprint_id}.json");

    let token = std::env::var(&config.github.token_env)
        .with_context(|| format!("Missing env var: {}", config.github.token_env))?;

    let (owner, repo) = config
        .project
        .repo
        .split_once('/')
        .context("Invalid repo format — expected 'owner/repo'")?;

    let github = GitHubClient::new(&token, owner, repo)?;

    let repo_root = std::env::current_dir()?;
    let socket_path = PathBuf::from(format!("/run/caloron/{sprint_id}.sock"));

    // Register project in global dashboard
    if let Err(e) = crate::dashboard::register_current_project(
        &config.project.name,
        &config.project.repo,
        &repo_root,
    ) {
        tracing::warn!(error = %e, "Failed to register project in dashboard");
    }

    let mut orchestrator = Orchestrator::new(config, dag, github, repo_root, socket_path);

    tracing::info!(sprint_id, "Daemon starting");
    orchestrator.run().await
}
