use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::Mutex;

use caloron_types::config::CaloronConfig;
use caloron_types::feedback::CaloronFeedback;

use crate::agent::spawner::AgentSpawner;
use crate::config;
use crate::dag::engine::DagEngine;
use crate::daemon::socket::DaemonSocket;
use crate::daemon::state::DaemonState;
use crate::git::GitHubClient;
use crate::git::monitor::{EventHandler, OrchestratorAction};
use crate::noether::client::NoetherService;
use crate::supervisor::health_monitor::{HealthMonitor, HealthMonitorConfig};
use crate::supervisor::interventions::{InterventionAction, InterventionDecider, InterventionTracker};

/// The main orchestration loop that ties all components together.
pub struct Orchestrator {
    config: CaloronConfig,
    state: DaemonState,
    dag: Arc<Mutex<DagEngine>>,
    github: GitHubClient,
    spawner: Arc<Mutex<AgentSpawner>>,
    health_monitor: HealthMonitor,
    intervention_tracker: InterventionTracker,
    socket_path: PathBuf,
    repo_root: PathBuf,
    /// Agent definitions loaded from meta repo, keyed by agent ID.
    agent_defs: HashMap<String, caloron_types::agent::AgentDefinition>,
    /// Credentials to inject into agents.
    credentials: HashMap<String, String>,
    /// Collected feedback for retro (persisted to disk).
    feedback_buffer: Vec<CaloronFeedback>,
    /// Noether service (if enabled).
    noether: Option<NoetherService>,
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

        // [Fix #5] Pass the actual socket path to the spawner so agents connect
        // to the right socket.
        let spawner = AgentSpawner::new(config.clone(), repo_root.clone(), socket_path.clone());

        let health_config = HealthMonitorConfig {
            check_interval: Duration::from_secs(config.github.polling_interval_seconds as u64),
            max_review_cycles: config.supervisor.max_review_cycles,
            ..Default::default()
        };

        // Collect credentials from environment for agent injection
        let mut credentials = HashMap::new();
        if let Ok(token) = std::env::var(&config.github.token_env) {
            credentials.insert("GITHUB_TOKEN".into(), token);
        }
        if let Ok(key) = std::env::var(&config.llm.api_key_env) {
            credentials.insert("ANTHROPIC_API_KEY".into(), key);
        }

        // [Fix #8] Initialize Noether if enabled
        let noether = if config.noether.enabled {
            let svc = NoetherService::new(&config.noether.endpoint);
            if svc.is_available() {
                tracing::info!("Noether service connected");
                Some(svc)
            } else {
                tracing::warn!("Noether enabled but not available");
                None
            }
        } else {
            None
        };

        Self {
            config,
            state,
            dag: Arc::new(Mutex::new(dag)),
            github,
            spawner: Arc::new(Mutex::new(spawner)),
            health_monitor: HealthMonitor::new(health_config),
            intervention_tracker: InterventionTracker::new(),
            socket_path,
            repo_root,
            agent_defs: HashMap::new(),
            credentials,
            feedback_buffer: Vec::new(),
            noether,
        }
    }

    /// Load agent definitions from the DAG — resolves specs via the agent generator,
    /// or falls back to loading YAML from definition_path.
    pub fn load_agent_definitions(&mut self) -> Result<()> {
        let dag = self.dag.try_lock().unwrap();
        let dag_snapshot = dag.state().clone();
        drop(dag); // release lock before I/O

        // Build the full DAG struct for the resolver
        let dag_for_resolver = caloron_types::dag::Dag {
            sprint: dag_snapshot.sprint.clone(),
            agents: dag_snapshot.agents.values().cloned().collect(),
            tasks: dag_snapshot.tasks.values().map(|ts| ts.task.clone()).collect(),
            review_policy: caloron_types::dag::ReviewPolicy {
                required_approvals: 1,
                auto_merge: true,
                max_review_cycles: self.config.supervisor.max_review_cycles,
            },
            escalation: caloron_types::dag::EscalationConfig {
                stall_threshold_minutes: self.config.supervisor.stall_default_threshold_minutes,
                supervisor_id: "supervisor".into(),
                human_contact: self.config.supervisor.escalation_method.clone(),
            },
        };

        match crate::kickoff::resolver::resolve_agents(&dag_for_resolver, &self.repo_root) {
            Ok(defs) => {
                tracing::info!(count = defs.len(), "Resolved agent definitions");
                self.agent_defs = defs;
            }
            Err(e) => {
                tracing::warn!(error = %e, "Agent resolution failed — falling back to defaults");
            }
        }

        Ok(())
    }

    /// Run the main orchestration loop.
    pub async fn run(&mut self) -> Result<()> {
        let sprint_id = {
            let dag = self.dag.lock().await;
            dag.sprint_id().to_string()
        };
        tracing::info!(sprint_id, "Starting orchestration loop");

        // Load saved feedback buffer
        self.load_feedback_buffer();

        // Ensure labels exist
        if let Err(e) = self.github.ensure_labels().await {
            tracing::warn!(error = %e, "Could not ensure labels — continuing anyway");
        }

        // [Fix #3] Sync DAG state to DaemonState for socket/dashboard queries
        self.sync_dag_to_state().await;

        // Spawn agents for initially ready tasks
        self.spawn_ready_tasks().await?;

        // Start the daemon socket for harness communication
        let socket = DaemonSocket::new(self.socket_path.clone(), self.state.clone());
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

            // [Fix #3] Keep DaemonState in sync after each event cycle
            self.sync_dag_to_state().await;

            // Run health checks
            self.run_health_checks().await;

            // Update dashboard
            if let Err(e) = crate::dashboard::update_sprint_summary(
                &self.repo_root,
                self.dag.lock().await.state(),
            ) {
                tracing::debug!(error = %e, "Failed to update dashboard");
            }

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

        // Persist feedback buffer on clean exit
        self.save_feedback_buffer();

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

            // [Fix #1] Actually spawn agents
            OrchestratorAction::SpawnAgent {
                task_id,
                agent_def_id,
                issue_number,
            } => {
                tracing::info!(task_id, agent_def_id, issue_number, "Spawning agent for task");

                let _ = self.github.add_label(issue_number, "caloron:assigned").await;
                let comment = format!("@caloron-agent-{agent_def_id} has been assigned this task.");
                let _ = self.github.create_comment(issue_number, &comment).await;

                // Get the agent definition (or create a minimal one)
                let agent_def = self.agent_defs.get(&agent_def_id).cloned().unwrap_or_else(|| {
                    tracing::warn!(agent_def_id, "No agent definition loaded — using minimal defaults");
                    caloron_types::agent::AgentDefinition {
                        name: agent_def_id.clone(),
                        version: "1.0".into(),
                        description: format!("Auto-generated for {agent_def_id}"),
                        llm: caloron_types::agent::LlmConfig {
                            model: self.config.llm.resolve_model("default"),
                            max_tokens: 8192,
                            temperature: 0.2,
                        },
                        system_prompt: format!("You are agent {agent_def_id}. Complete the task in your assigned GitHub issue."),
                        tools: vec!["bash".into(), "github_mcp".into()],
                        mcps: vec![],
                        nix: caloron_types::agent::NixConfig::default(),
                        credentials: vec!["GITHUB_TOKEN".into(), "ANTHROPIC_API_KEY".into()],
                        stall_threshold_minutes: self.config.supervisor.stall_default_threshold_minutes,
                        max_review_cycles: self.config.supervisor.max_review_cycles,
                    }
                });

                let sprint_id = self.dag.lock().await.sprint_id().to_string();

                // Spawn the agent process
                let mut spawner = self.spawner.lock().await;
                match spawner
                    .spawn(&agent_def_id, &agent_def, &task_id, &sprint_id, &self.credentials)
                    .await
                {
                    Ok(health) => {
                        // [Fix #4] Register agent in health map
                        self.state.register_agent(health).await;
                        tracing::info!(agent_def_id, task_id, "Agent spawned and registered");
                    }
                    Err(e) => {
                        tracing::error!(agent_def_id, task_id, error = %e, "Failed to spawn agent");
                    }
                }
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
                issue_number: _,
                message: _,
            } => {
                tracing::info!(agent_id, "Notifying agent");
                self.state
                    .update_agent(&agent_id, |health| {
                        health.record_git_event();
                    })
                    .await;
            }

            // [Fix #6] Store feedback in buffer and persist
            OrchestratorAction::StoreFeedback { task_id, feedback } => {
                tracing::info!(task_id, "Storing feedback for retro");
                self.feedback_buffer.push(feedback);
                self.save_feedback_buffer();
            }

            OrchestratorAction::TaskCompleted {
                task_id,
                issue_number,
                pr_number,
                unblocked,
            } => {
                tracing::info!(task_id, pr_number, "Task completed");

                let _ = self.github.add_label(issue_number, "caloron:done").await;
                let comment = format!("Completed via PR #{pr_number}");
                let _ = self.github.close_issue(issue_number, &comment).await;

                // Destroy the agent for the completed task
                let sprint_id = self.dag.lock().await.sprint_id().to_string();
                {
                    let dag = self.dag.lock().await;
                    if let Some(ts) = dag.state().tasks.get(&task_id) {
                        let agent_id = &ts.task.assigned_to;
                        let mut spawner = self.spawner.lock().await;
                        let _ = spawner.destroy(agent_id, &sprint_id).await;
                        self.state.unregister_agent(agent_id).await;
                    }
                }

                for unblocked_id in unblocked {
                    tracing::info!(unblocked_id, "Task unblocked — will spawn on next cycle");
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
        let ready: Vec<(String, String)> = {
            let dag = self.dag.lock().await;
            dag.get_ready_tasks()
                .iter()
                .map(|ts| (ts.task.id.clone(), ts.task.assigned_to.clone()))
                .collect()
        };

        for (task_id, agent_id) in ready {
            tracing::info!(task_id, agent_id, "Ready task — creating issue to trigger spawn");
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

            let action =
                InterventionDecider::decide(&result, &task_id, &self.intervention_tracker);

            tracing::warn!(
                agent_id = result.agent_id,
                verdict = ?result.verdict,
                intervention = ?action,
                "Health check failed"
            );

            self.intervention_tracker
                .record(&task_id, &result.agent_id, action.clone());

            // [Fix #9] Look up issue number from DAG for escalation
            let issue_number = {
                let dag = self.dag.lock().await;
                dag.state()
                    .tasks
                    .get(&task_id)
                    .and_then(|ts| ts.task.github_issue_number)
                    .unwrap_or(0)
            };

            match action {
                InterventionAction::Probe => {
                    if issue_number > 0 {
                        if let Some(health) = agents.get(&result.agent_id) {
                            let minutes =
                                (chrono::Utc::now() - health.last_git_event).num_minutes() as u64;
                            let comment = crate::supervisor::interventions::probe_comment(
                                &result.agent_id,
                                minutes,
                            );
                            let _ = self.github.create_comment(issue_number, &comment).await;
                        }
                    }
                }
                InterventionAction::Restart => {
                    tracing::info!(agent_id = result.agent_id, "Restarting agent");
                    let sprint_id = self.dag.lock().await.sprint_id().to_string();
                    if let Some(agent_def) = self.agent_defs.get(&result.agent_id) {
                        let mut spawner = self.spawner.lock().await;
                        match spawner
                            .restart(
                                &result.agent_id,
                                agent_def,
                                &task_id,
                                &sprint_id,
                                &self.credentials,
                            )
                            .await
                        {
                            Ok(health) => {
                                self.state.register_agent(health).await;
                                tracing::info!(agent_id = result.agent_id, "Agent restarted");
                            }
                            Err(e) => {
                                tracing::error!(
                                    agent_id = result.agent_id,
                                    error = %e,
                                    "Failed to restart agent"
                                );
                            }
                        }
                    }
                }
                // [Fix #9] Use actual issue number in escalation
                InterventionAction::EscalateCredentials { tool } => {
                    let escalation =
                        crate::supervisor::escalation::EscalationIssue::CredentialsFailure {
                            agent_role: result.agent_id.clone(),
                            tool,
                            task_issue_number: issue_number,
                            error_count: agents
                                .get(&result.agent_id)
                                .map(|h| h.consecutive_errors)
                                .unwrap_or(0),
                        };
                    let _ = crate::supervisor::escalation::EscalationGateway::escalate(
                        &self.github,
                        &escalation,
                    )
                    .await;
                }
                InterventionAction::EscalateCapability => {
                    let escalation =
                        crate::supervisor::escalation::EscalationIssue::TaskBeyondCapability {
                            agent_role: result.agent_id.clone(),
                            task_issue_number: issue_number,
                            task_title: task_id.clone(),
                            attempts: self
                                .intervention_tracker
                                .history_for_task(&task_id)
                                .len(),
                            failure_pattern: format!("{:?}", result.verdict),
                        };
                    let _ = crate::supervisor::escalation::EscalationGateway::escalate(
                        &self.github,
                        &escalation,
                    )
                    .await;
                }
                InterventionAction::EscalateReviewLoop { pr_id } => {
                    let escalation =
                        crate::supervisor::escalation::EscalationIssue::ReviewLoop {
                            pr_number: pr_id.parse().unwrap_or(0),
                            reviewer_id: result.agent_id.clone(),
                            author_id: "unknown".into(),
                            cycles: 0,
                            analysis: format!("{:?}", result.verdict),
                        };
                    let _ = crate::supervisor::escalation::EscalationGateway::escalate(
                        &self.github,
                        &escalation,
                    )
                    .await;
                }
                InterventionAction::Reassign { new_agent_id } => {
                    tracing::info!(
                        from = result.agent_id,
                        to = new_agent_id,
                        "Reassigning task"
                    );
                }
                InterventionAction::Block { reason } => {
                    tracing::warn!(agent_id = result.agent_id, reason, "Blocking task");
                    if !task_id.is_empty() {
                        let mut dag = self.dag.lock().await;
                        let _ = dag.task_blocked(&task_id, &reason);
                    }
                }
            }
        }
    }

    /// [Fix #3] Sync DAG state to DaemonState so socket/dashboard can read it.
    async fn sync_dag_to_state(&self) {
        let dag = self.dag.lock().await;
        self.state.set_dag(dag.state().clone()).await;
    }

    /// [Fix #6] Load feedback buffer from disk.
    fn load_feedback_buffer(&mut self) {
        let path = self.feedback_file_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    for line in content.lines() {
                        if let Ok(fb) = serde_json::from_str::<CaloronFeedback>(line) {
                            self.feedback_buffer.push(fb);
                        }
                    }
                    tracing::info!(
                        count = self.feedback_buffer.len(),
                        "Loaded feedback buffer"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to load feedback buffer");
                }
            }
        }
    }

    /// [Fix #6] Save feedback buffer to disk (JSONL format).
    fn save_feedback_buffer(&self) {
        let path = self.feedback_file_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content: String = self
            .feedback_buffer
            .iter()
            .filter_map(|fb| serde_json::to_string(fb).ok())
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = std::fs::write(&path, content) {
            tracing::warn!(error = %e, "Failed to save feedback buffer");
        }
    }

    fn feedback_file_path(&self) -> PathBuf {
        self.repo_root.join("state").join("feedback.jsonl")
    }
}

/// Entry point: load config and DAG, create orchestrator, and run.
pub async fn start_daemon(config_path: &Path, dag_path: &Path) -> Result<()> {
    let config = crate::config::load_config(config_path)?;

    // [Fix #2] Load DAG and set state file for persistence
    let mut dag = DagEngine::load_from_file(dag_path).context("Failed to load DAG")?;

    let sprint_id = dag.sprint_id().to_string();
    let state_file = format!("state/sprint-{sprint_id}.json");
    dag.set_state_file(&state_file);

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

    // Load agent definitions from the DAG
    if let Err(e) = orchestrator.load_agent_definitions() {
        tracing::warn!(error = %e, "Some agent definitions could not be loaded");
    }

    tracing::info!(sprint_id, state_file, "Daemon starting");
    orchestrator.run().await
}
